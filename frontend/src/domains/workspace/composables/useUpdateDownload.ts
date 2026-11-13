/**
 * useUpdateDownload：更新包「下载 → 安装 → 重启」状态机（关于弹窗 / 独立更新详情弹窗共用）。
 *
 * - Started 只带 contentLength，Progress 只带 chunkLength，须自行累加换算真实进度；
 * - 速度按 ≥300ms 间隔滑动平均采样（EMA），避免逐 chunk 抖动；
 * - Finished 后进入安装中（验签 / 解压 / 拉起安装器），安装完成 toast + 延迟 relaunch；
 * - 失败就地记录 failMsg 并 toast（附重试动作），不向外抛出，调用方按返回值收尾；
 * - 关闭 Update 对象、成功提示与重启均在本层完成，调用方只负责清本地展示状态。
 */
import { computed, ref, type ComputedRef, type Ref } from 'vue'
import { relaunch } from '@tauri-apps/plugin-process'
import type { Update } from '@tauri-apps/plugin-updater'
import { useToast } from '@/composables/useToast'
import { useLocaleStore } from '@/stores/locale'
import { errorText } from '@/utils/errorText'

export interface UpdateDownloadState {
  /** 是否在下载 / 安装中（下载中禁止关闭弹窗）。 */
  downloading: Ref<boolean>
  /** 下载进度（0-1）；null = 总量未知（走不确定态）。 */
  progress: Ref<number | null>
  /** 验签 / 解压 / 拉起安装器阶段（Finished 之后、重启之前）。 */
  installing: Ref<boolean>
  /** 就地失败信息（重试按钮由 downloading=false 自动恢复）。 */
  failMsg: Ref<string>
  /** 状态行：安装中 → 阶段文案；否则 百分比 · 已下/总量 · 速度 组合。 */
  statusText: ComputedRef<string>
  /** 执行下载安装；成功返回 true，失败就地记录并返回 false。 */
  start: (update: Update) => Promise<boolean>
}

/** 下载状态机（每个组件实例一份：弹窗各自持有进度互不串扰）。 */
export function useUpdateDownload(): UpdateDownloadState {
  const toast = useToast()
  const t = useLocaleStore().t

  const downloading = ref(false)
  const progress = ref<number | null>(null)
  const installing = ref(false)
  /** 已下载 / 总量字节（Started 的 contentLength 可能为 0 = 未知）。 */
  const downloadedBytes = ref(0)
  const totalBytes = ref(0)
  /** 平滑下载速度（MB/s，>=0.1 才展示，避免抖动噪音）。 */
  const speedMBs = ref(0)
  const failMsg = ref('')
  /** 最近一次速度采样时间与字节数（非响应式，仅计算用）。 */
  let speedLastAt = 0
  let speedLastBytes = 0

  const statusText = computed(() => {
    if (installing.value) return t('about.installing')
    const pct = progress.value != null ? `${Math.round(progress.value * 100)}%` : ''
    const done = (downloadedBytes.value / 1_000_000).toFixed(1)
    const size =
      downloadedBytes.value > 0
        ? totalBytes.value > 0
          ? t('about.sizeOf', { done, total: (totalBytes.value / 1_000_000).toFixed(1) })
          : t('about.sizeDone', { done })
        : ''
    const speed = speedMBs.value >= 0.1 ? t('about.speed', { v: speedMBs.value.toFixed(1) }) : ''
    const parts = [pct, size, speed].filter(Boolean)
    return parts.length ? parts.join(' · ') : t('about.downloading')
  })

  /** 滑动平均采样速度：≥300ms 采一次，避免逐 chunk 抖动。 */
  function sampleSpeed(): void {
    const now = Date.now()
    if (speedLastAt === 0) {
      speedLastAt = now
      speedLastBytes = downloadedBytes.value
      return
    }
    const dt = now - speedLastAt
    if (dt < 300) return
    const sample = ((downloadedBytes.value - speedLastBytes) / dt) * 1000 / 1_000_000
    speedLastAt = now
    speedLastBytes = downloadedBytes.value
    speedMBs.value = speedMBs.value > 0 ? speedMBs.value * 0.7 + sample * 0.3 : sample
  }

  /** 重新开始前清空上一轮状态（失败重试也走这里）。 */
  function reset(): void {
    failMsg.value = ''
    progress.value = null
    installing.value = false
    downloadedBytes.value = 0
    totalBytes.value = 0
    speedMBs.value = 0
    speedLastAt = 0
    speedLastBytes = 0
  }

  async function start(update: Update): Promise<boolean> {
    if (downloading.value) return false
    reset()
    downloading.value = true
    try {
      await update.downloadAndInstall((event) => {
        switch (event.event) {
          case 'Started':
            totalBytes.value = event.data.contentLength ?? 0
            downloadedBytes.value = 0
            progress.value = totalBytes.value > 0 ? 0 : null
            speedLastAt = 0
            speedLastBytes = 0
            speedMBs.value = 0
            break
          case 'Progress':
            // Progress 只带当前块长度（chunkLength），须自行累加才能表示真实进度
            downloadedBytes.value += event.data.chunkLength
            sampleSpeed()
            progress.value =
              totalBytes.value > 0 ? Math.min(downloadedBytes.value / totalBytes.value, 1) : null
            break
          case 'Finished':
            progress.value = 1
            installing.value = true
            break
        }
      })
      update.close()
      toast.success(t('about.installed'))
      setTimeout(() => relaunch(), 800)
      return true
    } catch (err) {
      const msg = errorText(err)
      installing.value = false
      progress.value = null
      failMsg.value = t('about.downloadFailed', { msg })
      toast.error(t('about.downloadFail'), {
        message: msg,
        action: { label: t('common.retry'), run: () => void start(update) },
      })
      return false
    } finally {
      downloading.value = false
    }
  }

  return { downloading, progress, installing, failMsg, statusText, start }
}
