/**
 * useLifecycleEditor：工作流编辑器的生命周期（setup / teardown）与数据驱动
 * 数据集编辑桥（自 WorkflowEditor 抽出，约 145 行）。
 *
 * 职责：三 Tab JSON 文本 ↔ 场景对象同步（lifecycleText / applyLifecycleText）、
 * 数据集数组的原地增删改（add/removeDataSet）、来源类型切换（setDsKind）、
 * inline 行的键值对扁平编辑（row* 系列）与 json/csv 文本（dsText）。
 * 全部操作直接写回场景对象（保存走既有 save 链路；解析失败不动数据）。
 */
import { computed, ref, watch, type Ref } from 'vue'
import type { DataSet, Scenario, ScenarioStep } from '@/types/miyoApi'
import { errorText } from '@/utils/errorText'

export interface UseLifecycleEditorOptions {
  /** 当前编辑场景（null = 未打开）。 */
  scenario: Ref<Scenario | null>
  /** 文案函数（组件 setup 内的 locale.t）。 */
  t: (key: string, params?: Record<string, string | number>) => string
}

export function useLifecycleEditor({ scenario, t }: UseLifecycleEditorOptions) {
  const lifecycleTab = ref<'setup' | 'teardown' | 'datasets'>('setup')
  const lifecycleText = ref('')
  const lifecycleError = ref<string | null>(null)

  function lifecycleJson(): string {
    const sc = scenario.value
    if (!sc) return '[]'
    if (lifecycleTab.value === 'datasets') return JSON.stringify(sc.data_sets ?? [], null, 2)
    return JSON.stringify(sc[lifecycleTab.value] ?? [], null, 2)
  }

  watch([lifecycleTab, scenario], () => {
    lifecycleText.value = lifecycleJson()
    lifecycleError.value = null
  })

  /** 解析并写回场景对象（保存走既有 save 链路；解析失败不动数据）。 */
  function applyLifecycleText(): boolean {
    const sc = scenario.value
    if (!sc) return false
    try {
      const parsed: unknown = JSON.parse(lifecycleText.value)
      if (lifecycleTab.value === 'datasets') {
        sc.data_sets = parsed as Scenario['data_sets']
      } else {
        sc[lifecycleTab.value] = parsed as ScenarioStep[]
      }
      lifecycleError.value = null
      return true
    } catch (err) {
      lifecycleError.value = errorText(err)
      return false
    }
  }

  // ---------- 数据集结构化编辑（datasets 数组的原地增删改） ----------
  const dsIndex = ref(0) // 当前编辑的数据集位次
  const dsCurrent = computed<DataSet | null>(
    () => scenario.value?.data_sets?.[dsIndex.value] ?? null,
  )

  function addDataSet(): void {
    const sc = scenario.value
    if (!sc) return
    const ds: DataSet = {
      id: crypto.randomUUID(),
      name: t('integration.dsAdd', { n: (sc.data_sets?.length ?? 0) + 1 }),
      source: { kind: 'inline', rows: [{}] },
      sort_order: sc.data_sets?.length ?? 0,
    }
    sc.data_sets = [...(sc.data_sets ?? []), ds]
    dsIndex.value = (sc.data_sets?.length ?? 1) - 1
  }

  function removeDataSet(): void {
    const sc = scenario.value
    if (!sc?.data_sets?.length) return
    sc.data_sets = sc.data_sets.filter((_, i) => i !== dsIndex.value)
    dsIndex.value = Math.min(dsIndex.value, (sc.data_sets?.length ?? 1) - 1)
  }

  const dsName = computed({
    get: () => dsCurrent.value?.name ?? '',
    set: (v: string) => {
      if (dsCurrent.value) dsCurrent.value.name = v
    },
  })

  const dsKind = computed(() => dsCurrent.value?.source.kind ?? 'inline')

  const dsKindOptions = [
    { value: 'inline', label: t('integration.dsKindInline') },
    { value: 'json', label: t('integration.dsKindJson') },
    { value: 'csv', label: t('integration.dsKindCsv') },
    { value: 'environment', label: t('integration.dsKindEnv') },
  ]

  function setDsKind(kind: string | number): void {
    const ds = dsCurrent.value
    if (!ds) return
    if (kind === 'inline') ds.source = { kind: 'inline', rows: [] }
    else if (kind === 'json') ds.source = { kind: 'json', text: '' }
    else if (kind === 'csv') ds.source = { kind: 'csv', text: '' }
    else ds.source = { kind: 'environment' }
  }

  /** inline 行编辑：增删行 + 键值对编辑（扁平字符串化）。 */
  const dsRows = computed(() => {
    const src = dsCurrent.value?.source
    return src && 'rows' in src ? src.rows : []
  })

  function addRow(): void {
    const ds = dsCurrent.value
    if (!ds || ds.source.kind !== 'inline') return
    ds.source.rows = [...ds.source.rows, {}]
  }

  function removeRow(i: number): void {
    const ds = dsCurrent.value
    if (!ds || ds.source.kind !== 'inline') return
    ds.source.rows = ds.source.rows.filter((_, k) => k !== i)
  }

  /** 行键值对（扁平 view：entry key 可编辑——新增键用 addRowKey）。 */
  function rowEntries(row: Record<string, unknown>): [string, string][] {
    return Object.entries(row).map(([k, v]) => [k, typeof v === 'string' ? v : JSON.stringify(v)])
  }

  function rowSetKey(row: Record<string, unknown>, oldKey: string, newKey: string): void {
    const trimmed = newKey.trim()
    if (!trimmed || trimmed === oldKey) return
    const entries = Object.entries(row).map(([k, v]) => (k === oldKey ? [trimmed, v] : [k, v]))
    for (const key of Object.keys(row)) delete row[key]
    Object.assign(row, Object.fromEntries(entries))
  }

  function rowSetValue(row: Record<string, unknown>, key: string, value: string): void {
    row[key] = value
  }

  function rowRemoveKey(row: Record<string, unknown>, key: string): void {
    delete row[key]
  }

  function rowAddKey(row: Record<string, unknown>): void {
    let k = 'field'
    let n = 1
    while (k in row) k = `field${++n}`
    row[k] = ''
  }

  const dsText = computed({
    get: () => {
      const src = dsCurrent.value?.source
      if (src && 'text' in src) return src.text
      return ''
    },
    set: (text: string) => {
      const ds = dsCurrent.value
      if (!ds) return
      if (ds.source.kind === 'json') ds.source = { kind: 'json', text }
      else if (ds.source.kind === 'csv') ds.source = { kind: 'csv', text }
    },
  })

  return {
    lifecycleTab,
    lifecycleText,
    lifecycleError,
    lifecycleJson,
    applyLifecycleText,
    dsIndex,
    dsCurrent,
    addDataSet,
    removeDataSet,
    dsName,
    dsKind,
    dsKindOptions,
    setDsKind,
    dsRows,
    addRow,
    removeRow,
    rowEntries,
    rowSetKey,
    rowSetValue,
    rowRemoveKey,
    rowAddKey,
    dsText,
  }
}
