//! 平台进程加固（spawn 时逐平台落地；隔离等级如实分级，不冒充沙箱）。
//!
//! | 平台 | 本模块实现 | 明确不提供 |
//! |---|---|---|
//! | unix 通用 | 新会话（setsid → 进程组 kill）、`RLIMIT_NOFILE` | 系统调用过滤 |
//! | linux | + `RLIMIT_AS` 内存硬限（Darwin 未实现 AS 限额——macOS 如实降级） | seccomp / cgroup v2（需特权） |
//! | windows | Job Object（KILL_ON_JOB_CLOSE + 进程内存上限） | AppContainer |
//! | macos | setsid + rlimit（sandbox-exec 已废弃，不引入） | seatbelt profile |
//! | linux | setsid + rlimit（Landlock 收口见 docs/plugin/SIDECAR.md 路线图） | seccomp / cgroup v2（需特权） |
//!
//! 全平台兜底：子进程以**最小环境变量**启动（仅必需项白名单），不继承宿主
//! 完整环境；IPC 通道仅 stdio。

use miyo_core::AppError;

/// sidecar 子进程的必需环境变量（在 env_clear 之上放行）。
const BASE_ENV_KEYS: &[&str] = &[
    "PATH",
    "HOME",
    "TMPDIR",
    "TEMP",
    "TMP",
    "LANG",
    "SYSTEMROOT",
];

/// 平台加固：spawn 前 configure + spawn 后 attach 保持的句柄。
pub struct PlatformGuard {
    #[cfg(target_os = "windows")]
    job: windows_job::JobHandle,
}

impl std::fmt::Debug for PlatformGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PlatformGuard")
    }
}

impl PlatformGuard {
    /// spawn 前配置：最小环境 + 进程组 + rlimit（unix）。
    pub fn configure(cmd: &mut tokio::process::Command, max_memory_mb: Option<u32>) {
        cmd.env_clear();
        for key in BASE_ENV_KEYS {
            if let Ok(value) = std::env::var(key) {
                cmd.env(key, value);
            }
        }
        configure_unix(cmd, max_memory_mb);
    }

    /// spawn 后关联：Windows Job Object（其余平台无 spawn 后句柄）。
    pub fn attach(child_pid: u32, max_memory_mb: Option<u32>) -> Result<PlatformGuard, AppError> {
        #[cfg(target_os = "windows")]
        {
            Ok(PlatformGuard {
                job: windows_job::JobHandle::assign(child_pid, max_memory_mb)?,
            })
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = (child_pid, max_memory_mb);
            Ok(PlatformGuard {})
        }
    }

    /// 终止平台侧句柄（windows：TerminateJobObject；其余平台无操作——
    /// unix 终止走 [`kill_process_group`]）。
    pub fn terminate(&self) {
        #[cfg(target_os = "windows")]
        {
            self.job.terminate();
        }
        #[cfg(not(target_os = "windows"))]
        {}
    }
}

/// 终止整个进程组（unix：setsid 后 pid == pgid，kill(-pid) 覆盖全部子孙；
/// windows 由 Job Object KILL_ON_JOB_CLOSE / terminate 承担）。
pub fn kill_process_group(pid: u32) {
    #[cfg(unix)]
    {
        // SAFETY：kill 系统调用；负数 pid 语义 = 指定进程组
        unsafe {
            let _ = libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
    }
}

#[cfg(unix)]
fn configure_unix(cmd: &mut tokio::process::Command, max_memory_mb: Option<u32>) {
    // macOS 不支持 setrlimit(RLIMIT_AS)（Darwin 未实现，恒 EINVAL——实测），
    // 内存硬限仅在 Linux 生效；macOS 侧如实降级（见模块头隔离矩阵）。
    #[cfg(target_os = "macos")]
    let memory: Option<u64> = None;
    #[cfg(not(target_os = "macos"))]
    let memory = max_memory_mb.map(|mb| mb as u64 * 1024 * 1024);
    let _ = max_memory_mb;
    // SAFETY：pre_exec 在 fork 后 exec 前运行；仅调用 setsid / setrlimit，
    // 不分配内存、不执行任意代码。
    unsafe {
        cmd.pre_exec(move || {
            // 新会话：子进程成为组长（pid == pgid），终止时可 kill(-pgid)
            // 覆盖其派生的全部子孙；失败不阻断（直接 kill pid 兜底）。
            let _ = libc::setsid();
            if let Some(bytes) = memory {
                let lim = libc::rlimit {
                    rlim_cur: bytes,
                    rlim_max: bytes,
                };
                if libc::setrlimit(libc::RLIMIT_AS, &lim) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            let nofile = libc::rlimit {
                rlim_cur: 256,
                rlim_max: 256,
            };
            if libc::setrlimit(libc::RLIMIT_NOFILE, &nofile) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(not(unix))]
fn configure_unix(_cmd: &mut tokio::process::Command, _max_memory_mb: Option<u32>) {}

// ---------------------------------------------------------------------------
// Windows Job Object（仅 windows 目标编译；本地开发为 macOS，编译验证由
// CI windows job 承担）
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
mod windows_job {
    use miyo_core::AppError;

    pub struct JobHandle {
        handle: windows_sys::Win32::Foundation::HANDLE,
    }

    // HANDLE 跨线程使用（Supervisor 在 async 上下文持有）
    unsafe impl Send for JobHandle {}
    unsafe impl Sync for JobHandle {}

    impl JobHandle {
        /// 创建 Job（KILL_ON_JOB_CLOSE + 进程内存上限）并关联子进程。
        pub fn assign(pid: u32, max_memory_mb: Option<u32>) -> Result<JobHandle, AppError> {
            use windows_sys::Win32::Foundation::CloseHandle;
            use windows_sys::Win32::System::JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
                SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
            };
            unsafe {
                let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if job.is_null() {
                    return Err(AppError::Io(std::io::Error::other("CreateJobObjectW 失败")));
                }
                let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                if let Some(mb) = max_memory_mb {
                    // windows-sys 0.59：ProcessMemoryLimit 为 usize（指针宽度）
                    limits.ProcessMemoryLimit = mb as usize * 1024 * 1024;
                    limits.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_PROCESS_MEMORY;
                }
                let ok = SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &limits as *const _ as *const std::ffi::c_void,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                if ok == 0 {
                    CloseHandle(job);
                    return Err(AppError::Io(std::io::Error::other(
                        "SetInformationJobObject 失败",
                    )));
                }
                let proc = windows_sys::Win32::System::Threading::OpenProcess(
                    0x0100 | 0x0001, // PROCESS_SET_QUOTA | PROCESS_TERMINATE
                    0,
                    pid,
                );
                if proc.is_null() {
                    CloseHandle(job);
                    return Err(AppError::Io(std::io::Error::other(format!(
                        "OpenProcess({pid}) 失败（权限不足或进程已退出）"
                    ))));
                }
                let ok = AssignProcessToJobObject(job, proc);
                CloseHandle(proc);
                if ok == 0 {
                    CloseHandle(job);
                    return Err(AppError::Io(std::io::Error::other(
                        "AssignProcessToJobObject 失败",
                    )));
                }
                Ok(JobHandle { handle: job })
            }
        }

        /// 终止 Job 内全部进程（含 sidecar 派生的子孙）。
        pub fn terminate(&self) {
            use windows_sys::Win32::System::JobObjects::TerminateJobObject;
            unsafe {
                TerminateJobObject(self.handle, 1);
            }
        }
    }

    impl Drop for JobHandle {
        fn drop(&mut self) {
            // KILL_ON_JOB_CLOSE：句柄关闭即终止全部关联进程
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(self.handle);
            }
        }
    }
}
