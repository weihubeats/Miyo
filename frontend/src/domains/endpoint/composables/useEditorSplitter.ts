/**
 * useEditorSplitter：请求区 / 响应区高度分割状态机（自 EndpointEditor 抽出）。
 *
 * 职责（原 1091-1385 行整段，行为不变）：
 * - 分割态**按接口 id 分桶**（各自独立 + 记住手动拖过的高度，落 localStorage）；
 * - 未手动拖过时请求卡「贴内容 grow」布局，拖动基准取实测盒高（首拖不跳变）；
 * - grow chrome 按接口拆存（切接口重测，防跨接口钳错上下限）；
 * - 拖拽 / 双击折叠 / 键盘微调三套交互。
 *
 * 计数边界：`hasResponse` 由宿主提供（发送中 / 已有响应 / gRPC 流任一为真）；
 * `draftId` 仅用于分桶与 chrome 归属，宿主对它的其他 watch 不受影响。
 */
import { computed, onBeforeUnmount, ref, watch, type Ref } from 'vue'

export const REQUEST_MIN = 80
export const REQUEST_DEFAULT = 200
/** 请求区最大高度 = 编辑器高度 - MAX_OFFSET，保证响应区至少留 MAX_OFFSET px。 */
export const MAX_OFFSET = 100

/** 请求区最大高度兜底：量不到编辑器高度时使用（首帧 / 未排版环境）。 */
const EDITOR_FALLBACK_HEIGHT = 600
/** 手动高度生效下限：低于它就当作「折叠态」处理（用户拉塌的本意）。 */
const REQUEST_COLLAPSE_THRESHOLD = 40

/** 每个接口一条手动分割态：高度 + 是否手动拖过（同键写入，一并替换）。 */
interface SplitState {
  height: number
  manual: boolean
}
/** 桶上限：只存数字 + 布尔，超限按插入顺序淘汰最旧的（当前接口除外）。 */
const SPLIT_BUCKET_MAX = 200
const SPLIT_STORE_KEY = 'miyo.request-split-by-endpoint'
const SPLIT_STORE_LIMIT = 200

export interface UseEditorSplitterOptions {
  draftId: Ref<string | null | undefined>
  hasResponse: Ref<boolean>
  /** 编辑器根元素：量总高用。 */
  editorEl: Ref<HTMLElement | null>
  /** 请求卡（config-box）：grow 态实测盒高基准。 */
  configBoxEl: Ref<HTMLElement | null>
}

export function useEditorSplitter(options: UseEditorSplitterOptions) {
  const { draftId, hasResponse, editorEl, configBoxEl } = options

  /**
   * 分割桶为**实例级**（随组件卸载丢弃），跨重建的持久化由 localStorage 承担：
   * 工厂执行时读一次上次会话的桶（视图整棵重建后各接口沿用自己拖过的高度）。
   * 用块作用域而非 `;` 前置的 IIFE：后者紧跟在数值常量之后会被解析成调用表达式。
   */
  const splitBuckets = new Map<string, SplitState>()
  {
    try {
      const raw = localStorage.getItem(SPLIT_STORE_KEY)
      if (raw) {
        const parsed = JSON.parse(raw) as Record<string, SplitState>
        for (const [id, item] of Object.entries(parsed)) {
          if (typeof item?.height !== 'number' || !Number.isFinite(item.height)) continue
          splitBuckets.set(id, { height: item.height, manual: item.manual === true })
        }
      }
    } catch {
      /* 存储损坏 / 隐私模式：按默认布局走，不阻断编辑器挂载 */
    }
  }

  function persistSplitBuckets(keepId: string): void {
    try {
      // 只留最近 SPLIT_STORE_LIMIT 条，避免 localStorage 无限增长
      while (splitBuckets.size > SPLIT_STORE_LIMIT) {
        const oldest = splitBuckets.keys().next().value
        if (oldest === undefined || oldest === keepId) break
        splitBuckets.delete(oldest)
      }
      localStorage.setItem(SPLIT_STORE_KEY, JSON.stringify(Object.fromEntries(splitBuckets)))
    } catch {
      /* 配额 / 隐私模式写入失败：仅丢持久化，不影响本次会话 */
    }
  }

  const splitTick = ref(0)

  /** 当前接口的分割态（无记录 = 从未拖过，走 grow 贴内容布局）。 */
  function currentSplit(): SplitState | null {
    const id = draftId.value
    return id ? (splitBuckets.get(id) ?? null) : null
  }

  const requestBodyHeight = computed(() => {
    void splitTick.value
    return currentSplit()?.height ?? REQUEST_DEFAULT
  })
  /**
   * 未发送期手动拖过分割条：请求卡从「贴内容 grow」切为分割条分配的固定高。
   * 阈值用 40 而非 REQUEST_MIN(80)：拖到 80 以下也是用户明确选的「塌下去」，
   * 若按 80 判定就会回到 grow 贴内容布局，看起来正是「高度重置」。
   */
  const requestManual = computed(() => {
    void splitTick.value
    const cur = currentSplit()
    return !!cur?.manual && cur.height >= REQUEST_COLLAPSE_THRESHOLD
  })
  const requestBodyCollapsed = computed(() => {
    void splitTick.value
    return requestBodyHeight.value <= REQUEST_MIN
  })

  /** 写**当前接口**的请求卡固定高（无接口时丢弃：组件未渲染 v-if="draft"）。 */
  function setRequestBodyHeight(px: number): void {
    const id = draftId.value
    if (!id) return
    const prev = splitBuckets.get(id)
    splitBuckets.delete(id) // 重插到末尾 = 刷新「最近使用」序
    splitBuckets.set(id, { height: px, manual: prev?.manual ?? false })
    while (splitBuckets.size > SPLIT_BUCKET_MAX) {
      const oldest = splitBuckets.keys().next().value
      if (oldest === undefined || oldest === id) break
      splitBuckets.delete(oldest)
    }
    splitTick.value++
    persistSplitBuckets(id)
  }

  /** 标记**当前接口**已进入手动分割态。 */
  function setRequestManual(): void {
    const id = draftId.value
    if (!id) return
    const prev = splitBuckets.get(id)
    splitBuckets.delete(id)
    splitBuckets.set(id, { height: prev?.height ?? REQUEST_DEFAULT, manual: true })
    splitTick.value++
    persistSplitBuckets(id)
  }

  const splitterDragging = ref(false)

  let splitStartY = 0
  let splitStartHeight = 0
  let splitDragging = false

  /** growChrome 按接口拆存：切接口后再拖该接口时，chrome 才能按「仍在 grow 态」重测，
   * 否则会误用上一接口测得的 chrome；-1 表示当前接口尚未测量。 */
  const growChromeById = new Map<string, number>()
  let growChromeFor: string | null = null
  let hintMinH = 0

  /** 取当前接口的 chrome 测量值（-1 未测）。 */
  function currentGrowChrome(): number {
    const id = draftId.value ?? null
    if (id !== growChromeFor) return -1
    return growChromeById.get(id ?? '') ?? -1
  }

  /** 写入当前接口的 chrome 测量值。 */
  function setGrowChrome(v: number): void {
    const id = draftId.value ?? null
    growChromeFor = id
    if (id) growChromeById.set(id, v)
  }

  /**
   * 切接口时**不**清手动分割态（分割态按接口 id 分桶各自恢复）；这里只切
   * 「chrome 测量值归属的接口」：各接口布局不同，跨接口复用测量值会把
   * 请求卡钳到错误的上下限。
   */
  watch(
    () => draftId.value,
    (id) => {
      growChromeFor = id ?? null
    },
  )

  function requestMaxHeight(): number {
    const measured = editorEl.value?.clientHeight ?? 0
    const edH = measured > 0 ? measured : EDITOR_FALLBACK_HEIGHT
    const chrome = currentGrowChrome()
    if (!hasResponse.value && chrome >= 0) {
      return Math.max(edH - chrome - hintMinH, REQUEST_MIN + 40)
    }
    return Math.max(edH - MAX_OFFSET, REQUEST_MIN + 40)
  }

  /** 首次出现响应区时初始化分割：请求区约 35%（优先放大响应展示空间）。
   *  仅在 无响应→有响应 跳变时计算；同一响应会话内的手动拖拽在重发时不受
   *  影响（hasResponse 保持 true 不再触发）。已手动分桶过的接口不覆盖。 */
  watch(hasResponse, (has) => {
    if (!has) return
    // 该接口手动拖过高度：一律不覆盖（那是用户亲手定的值）。
    if (requestManual.value) return
    const available = editorEl.value?.clientHeight ?? 0
    if (available <= 0) return
    setRequestBodyHeight(
      Math.min(Math.max(Math.round(available * 0.35), REQUEST_DEFAULT), requestMaxHeight()),
    )
  })

  /** 未手动拖过分割条时，请求卡「贴内容」的实际高度（grow 布局下的盒高）。 */
  function measuredGrowHeight(): number {
    const actual = configBoxEl.value?.getBoundingClientRect().height
    if (actual === undefined || actual <= 0) return REQUEST_DEFAULT
    // border-box：用 getBoundingClientRect（含边框）与 height 样式同口径
    return Math.max(Math.min(Math.round(actual), requestMaxHeight()), REQUEST_MIN)
  }

  /** 进入手动分割模式（未发送期）：先以实际盒高对齐基准再切固定高，否则首次
   *  拖动会从旧值（200）跳变。已有响应 / 已是固定高时直接返回现值。 */
  function enterManualSplit(): number {
    if (hasResponse.value || requestManual.value) return requestBodyHeight.value
    const ed = editorEl.value
    const cb = configBoxEl.value
    const hint = ed?.querySelector<HTMLElement>('.response-hint')
    // grow 态 flex 收敛必不溢出：此刻量 chrome = 总高 − 请求卡 − 占位条
    if (ed && cb && hint) {
      setGrowChrome(
        ed.scrollHeight - cb.getBoundingClientRect().height - hint.getBoundingClientRect().height,
      )
      hintMinH = parseFloat(getComputedStyle(hint).minHeight) || hint.offsetHeight
    }
    const measured = measuredGrowHeight()
    setRequestManual()
    setRequestBodyHeight(measured)
    // 返回本次实测值：调用方在同一调用栈内据此判定折叠态，不隔着 computed 的缓存
    return measured
  }

  function onSplitterMove(event: MouseEvent): void {
    if (!splitDragging) return
    const next = splitStartHeight + (event.clientY - splitStartY)
    setRequestManual()
    // 下限先钳再取上限：反序时 Math.max(min, max(...)) 会把低于下限的值又抬回 min，
    // 拖到折叠区本该停在 REQUEST_MIN，却会反弹成 REQUEST_DEFAULT（折叠态永远拖不到）。
    setRequestBodyHeight(Math.max(Math.min(next, requestMaxHeight()), REQUEST_MIN))
  }

  function onSplitterUp(): void {
    if (!splitDragging) return
    splitDragging = false
    splitterDragging.value = false
    document.body.style.userSelect = ''
    document.removeEventListener('mousemove', onSplitterMove)
    document.removeEventListener('mouseup', onSplitterUp)
  }

  /** 分割条 mousedown：开始拖拽，动态调整请求区高度（响应区 flex:1 自动补位）。 */
  function onSplitterDown(event: MouseEvent): void {
    if (event.button !== 0) return
    event.preventDefault()
    splitDragging = true
    splitterDragging.value = true
    splitStartY = event.clientY
    // 基准必须取「此刻眼睛看到的盒高」：本接口从未拖过时存量值是默认的 200，
    // 与 grow 布局的实际高度不一致，直接用会让首拖跳变。固定高态下两者相同。
    splitStartHeight = hasResponse.value || requestManual.value
      ? requestBodyHeight.value
      : measuredGrowHeight()
    document.body.style.userSelect = 'none'
    document.addEventListener('mousemove', onSplitterMove)
    document.addEventListener('mouseup', onSplitterUp)
  }

  /** 双击分割条 / 点击微调按钮：请求区收缩到最小高度，再点恢复默认高度。 */
  function toggleRequestBody(): void {
    // 「折叠」判定用 requestBodyCollapsed 会漏掉用户把高度拖到 40~120 的情形，
    // 这里直接按固定高 + 实际高度判断，双击语义保持「塌到底 ⇄ 回到默认」。
    const wasCollapsed = !requestManual.value || requestBodyHeight.value <= REQUEST_MIN
    enterManualSplit()
    setRequestBodyHeight(wasCollapsed ? REQUEST_DEFAULT : REQUEST_MIN)
  }

  /** 键盘调高（分割条可 Tab 聚焦）：↑ ↓ 每次 10px，Shift 40px，PgUp/PgDn 更大步长，Home/End 到上下限。 */
  function onSplitterKeydown(event: KeyboardEvent): void {
    const handled = ['ArrowUp', 'ArrowDown', 'PageUp', 'PageDown', 'Home', 'End']
    if (!handled.includes(event.key)) return
    event.preventDefault()
    const max = requestMaxHeight()
    const step = event.shiftKey ? 40 : 10
    let next = enterManualSplit()
    if (event.key === 'ArrowUp') next -= step
    else if (event.key === 'ArrowDown') next += step
    else if (event.key === 'PageUp') next -= step * 4
    else if (event.key === 'PageDown') next += step * 4
    else if (event.key === 'Home') next = REQUEST_MIN
    else if (event.key === 'End') next = max
    setRequestManual()
    setRequestBodyHeight(Math.min(Math.max(next, REQUEST_MIN), max))
  }

  onBeforeUnmount(onSplitterUp)

  return {
    requestBodyHeight,
    requestManual,
    requestBodyCollapsed,
    splitterDragging,
    onSplitterDown,
    onSplitterMove,
    onSplitterUp,
    onSplitterKeydown,
    toggleRequestBody,
    enterManualSplit,
    measuredGrowHeight,
    requestMaxHeight,
  }
}
