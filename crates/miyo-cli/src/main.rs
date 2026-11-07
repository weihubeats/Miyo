//! miyo-cli 入口：clap 参数解析 + 人读输出 + 退出码。
//!
//! 退出码约定：0 全部通过 / 1 存在失败 / 2 输入错误（文件不存在、解析失败等）。

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use miyo_cli::{
    junit, load_backup, run_collection, run_scenario_file, RunOptions, ScenarioRunOptions,
};

#[derive(Parser)]
#[command(
    name = "miyo-cli",
    version,
    about = "Miyo 无头测试运行器：在终端 / CI 运行项目备份中的 API 测试集合",
    after_help = "集合文件即桌面端「设置 → 备份与恢复」导出的项目 JSON。"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// 运行测试集合（只运行带内嵌测试的接口，口径与桌面端一致）
    Run {
        /// 集合文件：桌面端导出的项目备份 JSON
        backup: PathBuf,
        /// 选择环境（取该环境的 Base URL 与变量，名称精确匹配）
        #[arg(long)]
        env: Option<String>,
        /// 覆盖 Base URL（优先级高于 --env）
        #[arg(long)]
        base_url: Option<String>,
        /// 只运行名称包含该子串的接口（大小写不敏感，可多次传入取并集）
        #[arg(long = "endpoint")]
        endpoints: Vec<String>,
        /// 注入变量 k=v（可多次，优先级最高）
        #[arg(long = "var")]
        vars: Vec<String>,
        /// 单请求超时（毫秒）
        #[arg(long)]
        timeout_ms: Option<u64>,
        /// 输出 JUnit XML 报告到该路径
        #[arg(long)]
        junit: Option<PathBuf>,
        /// 任一接口失败立即停止
        #[arg(long)]
        fail_fast: bool,
    },
    /// 运行集成测试场景（CI/CD：桌面端「集成测试 → 导出 JSON」的文件）
    Scenario {
        /// 场景文件（miyo.scenario 导出 JSON；引用步骤已自动物化为内联请求）
        scenario: PathBuf,
        /// 注入变量 k=v（可多次，优先级最高；base_url 由此或 --base-url 提供）
        #[arg(long = "var")]
        vars: Vec<String>,
        /// 覆盖 {{base_url}} 变量值（优先级高于 --var base_url=...）
        #[arg(long)]
        base_url: Option<String>,
        /// 单步超时（毫秒）
        #[arg(long)]
        timeout_ms: Option<u64>,
        /// 输出 JUnit XML 报告到该路径
        #[arg(long)]
        junit: Option<PathBuf>,
        /// 失败即停（任一步失败不再执行后续步骤）
        #[arg(long)]
        stop_on_failure: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Commands::Run {
            backup,
            env,
            base_url,
            endpoints,
            vars,
            timeout_ms,
            junit: junit_path,
            fail_fast,
        } => {
            let mut parsed_vars: Vec<(String, String)> = Vec::new();
            for pair in &vars {
                match pair.split_once('=') {
                    Some((k, v)) if !k.trim().is_empty() => {
                        parsed_vars.push((k.trim().to_string(), v.to_string()))
                    }
                    _ => {
                        eprintln!("无效的 --var（应为 k=v）：{pair}");
                        return ExitCode::from(2);
                    }
                }
            }
            let file = match load_backup(&backup) {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("{e}");
                    return ExitCode::from(2);
                }
            };

            let opts = RunOptions {
                env_name: env,
                base_url_override: base_url,
                endpoint_filters: endpoints,
                vars: parsed_vars,
                timeout_ms,
                fail_fast,
            };
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime 构建失败");
            let report = rt.block_on(run_collection(file, &opts));

            println!("▶ 项目：{}", report.project);
            for case in &report.cases {
                match &case.kind {
                    miyo_cli::CaseKind::Passed {
                        status,
                        duration_ms,
                        assertions,
                    } => {
                        let ms = duration_ms.map(|d| format!("{d:.0}ms")).unwrap_or_default();
                        println!(
                            "  ✓ {} {} — {} · {ms} · 断言 {}/{}",
                            case.method,
                            case.path,
                            status.map(|s| s.to_string()).unwrap_or_default(),
                            assertions.0,
                            assertions.1
                        );
                    }
                    miyo_cli::CaseKind::Failed {
                        status,
                        duration_ms: _,
                        failures,
                        request_error,
                    } => {
                        println!(
                            "  ✗ {} {} — {}",
                            case.method,
                            case.path,
                            status
                                .map(|s| s.to_string())
                                .unwrap_or_else(|| "请求失败".into())
                        );
                        if let Some(err) = request_error {
                            println!("      请求错误：{err}");
                        }
                        for (desc, reason) in failures {
                            match reason {
                                Some(r) => println!("      ✗ {desc}：{r}"),
                                None => println!("      ✗ {desc}"),
                            }
                        }
                    }
                    miyo_cli::CaseKind::Skipped { reason } => {
                        println!("  ⊘ {} {} — 跳过：{reason}", case.method, case.path);
                    }
                }
            }
            println!(
                "汇总：{} 通过 / {} 失败 / {} 跳过（{} 个接口，耗时 {:.1}s）",
                report.passed,
                report.failed,
                report.skipped,
                report.total_endpoints,
                report.total_duration_ms / 1000.0,
            );

            if let Some(path) = junit_path {
                match junit::write_junit(&report, &path) {
                    Ok(()) => println!("JUnit 报告已写入 {}", path.display()),
                    Err(e) => {
                        eprintln!("{e}");
                        return ExitCode::from(2);
                    }
                }
            }
            if report.ok() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Commands::Scenario {
            scenario,
            vars,
            base_url,
            timeout_ms,
            junit: junit_path,
            stop_on_failure,
        } => {
            let mut parsed_vars: Vec<(String, String)> = Vec::new();
            for pair in &vars {
                match pair.split_once('=') {
                    Some((k, v)) if !k.trim().is_empty() => {
                        parsed_vars.push((k.trim().to_string(), v.to_string()))
                    }
                    _ => {
                        eprintln!("无效的 --var（应为 k=v）：{pair}");
                        return ExitCode::from(2);
                    }
                }
            }
            let opts = ScenarioRunOptions {
                vars: parsed_vars,
                base_url_override: base_url,
                timeout_ms,
                stop_on_failure,
            };
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime 构建失败");
            let report = match rt.block_on(run_scenario_file(&scenario, &opts)) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("{e}");
                    return ExitCode::from(2);
                }
            };

            println!("▶ 场景：{}", report.project);
            for case in &report.cases {
                match &case.kind {
                    miyo_cli::CaseKind::Passed {
                        status,
                        duration_ms,
                        assertions,
                    } => {
                        let ms = duration_ms.map(|d| format!("{d:.0}ms")).unwrap_or_default();
                        println!(
                            "  ✓ {} {} — {} · {ms} · 断言 {}/{}",
                            case.method,
                            case.path,
                            status.map(|s| s.to_string()).unwrap_or_default(),
                            assertions.0,
                            assertions.1
                        );
                    }
                    miyo_cli::CaseKind::Failed {
                        status,
                        duration_ms: _,
                        failures,
                        request_error,
                    } => {
                        println!(
                            "  ✗ {} {} — {}",
                            case.method,
                            case.path,
                            status
                                .map(|s| s.to_string())
                                .unwrap_or_else(|| "请求失败".into())
                        );
                        if let Some(err) = request_error {
                            println!("      错误：{err}");
                        }
                        for (desc, reason) in failures {
                            match reason {
                                Some(r) => println!("      ✗ {desc}：{r}"),
                                None => println!("      ✗ {desc}"),
                            }
                        }
                    }
                    miyo_cli::CaseKind::Skipped { reason } => {
                        println!("  ⊘ {} {} — 跳过：{reason}", case.method, case.path);
                    }
                }
            }
            println!(
                "汇总：{} 通过 / {} 失败 / {} 跳过（{} 步，耗时 {:.1}s）",
                report.passed,
                report.failed,
                report.skipped,
                report.total_endpoints,
                report.total_duration_ms / 1000.0,
            );

            if let Some(path) = junit_path {
                match junit::write_junit(&report, &path) {
                    Ok(()) => println!("JUnit 报告已写入 {}", path.display()),
                    Err(e) => {
                        eprintln!("{e}");
                        return ExitCode::from(2);
                    }
                }
            }
            if report.ok() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
    }
}
