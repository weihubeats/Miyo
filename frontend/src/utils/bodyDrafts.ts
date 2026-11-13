/**
 * bodyDrafts：Body 类型页签各自的草稿（前端态，按接口隔离）。
 *
 * 语义：页签是数据的**分区**，不是同一份数据的多种视图——用户在哪个页签填的
 * 参数就住在哪个页签的草稿里，发送时用的恒是「当前选中页签的那一份」。所以
 * 切换类型只换 `request.body` 的引用，不改写别的页签的数据，也不做「切回来
 * 再还原」的推导/回填特殊处理（推导只会给出空 json 文本或空字段表，正是
 * 「输入框里有参数、带出去的却不对」的来源）。
 *
 * 存模块作用域而非组件内：BodyPanel 挂在 EndpointEditor 的「Body」配置页签下，
 * 切到 Params / Headers 等页签即卸载重建，组件内状态会随之丢失。
 *
 * Content-Type 也算页签自己的数据：它是用户/导入带来的真实请求头（如 cURL 导入的
 * `--data-raw` + `application/x-www-form-urlencoded`），切类型时只归档与还原，
 * 不按模式默认值改写——否则切回来 MIME 被换成 text/plain，服务端就不认了。
 *
 * 仅会话内记忆，不写进后端 BodySpec 契约；限制记住的接口数避免无限堆积。
 */
import { contentTypeOf, removeContentType, syncContentType, tabOf, rawMime, rawSubtypeOf } from '@/utils/bodyMode'
import type { BodyTab, RawSubtype } from '@/utils/bodyMode'
import type { BodySpec, Endpoint, GraphQLSpec, KeyValue } from '@/types/miyoApi'

/** 某个接口各页签的草稿 + raw 子类型。`slots` 里存的就是活对象（= 该页签激活时的 request.body）。 */
export interface BodyDrafts {
  rawSubtype: RawSubtype
  slots: Partial<Record<BodyTab, BodySpec>>
  /** 各页签离开时的 Content-Type 值（null / 缺失 = 当时没有该头）。 */
  contentTypes: Partial<Record<BodyTab, string | null>>
}

/** 最多记住多少个接口的 body 草稿（FIFO 淘汰最久未访问的）。 */
const MAX_ENDPOINTS = 100

const drafts = new Map<string, BodyDrafts>()

/** 写入即视为「最近使用」：删后重插保持 Map 插入序 = 使用序，超限淘汰最早的。 */
function touch(id: string, value: BodyDrafts): void {
  drafts.delete(id)
  drafts.set(id, value)
  while (drafts.size > MAX_ENDPOINTS) {
    drafts.delete(drafts.keys().next().value as string)
  }
}

/** 某页签的空白草稿（首次切到该类型时用）。 */
function blankBody(tab: BodyTab, rawSubtype: RawSubtype): BodySpec {
  switch (tab) {
    case 'none':
      return { mode: 'none' }
    case 'form-data':
      return { mode: 'multipart', fields: [] }
    case 'x-www-form-urlencoded':
      return { mode: 'urlencoded', fields: [] }
    case 'raw':
      return rawSubtype === 'json' ? { mode: 'json', raw: '' } : { mode: 'text', raw: '' }
    case 'binary':
      return { mode: 'binary', path: '' }
    case 'graphql':
      return {
        mode: 'graphql',
        spec: { query: '', variables: '{}', operation_name: '' } as GraphQLSpec,
      }
  }
}

/** 取（必要时创建）某接口的草稿表：首次访问按接口当前持久化的 body 归位。 */
export function draftsOf(endpoint: Endpoint): BodyDrafts {
  const cached = drafts.get(endpoint.id)
  if (cached) return cached
  const body = endpoint.request.body as BodySpec
  const headers = endpoint.request.headers ?? []
  const created: BodyDrafts = {
    rawSubtype: rawSubtypeOf(body, headers),
    slots: { [tabOf(body, headers)]: body },
    contentTypes: { [tabOf(body, headers)]: contentTypeOf(headers) || null },
  }
  touch(endpoint.id, created)
  return created
}

/** 取某页签的草稿（无则建空草稿并登记），返回值即要赋给 request.body 的对象。 */
export function slotOf(endpoint: Endpoint, tab: BodyTab, source: BodyDrafts = draftsOf(endpoint)): BodySpec {
  const existing = source.slots[tab]
  if (tab === 'raw') {
    // raw 的模式由子类型决定（json / text），文本跨子类型共用
    const mode = source.rawSubtype === 'json' ? 'json' : 'text'
    const raw = existing && 'raw' in existing ? existing.raw : ''
    if (existing && existing.mode === mode) return existing
    const next: BodySpec = { mode, raw }
    source.slots.raw = next
    return next
  }
  if (existing) return existing
  const created = blankBody(tab, source.rawSubtype)
  source.slots[tab] = created
  return created
}

/** 登记某页签的活对象（切走时把当前 request.body 归档，或外部整体替换后同步）。 */
export function setSlot(endpoint: Endpoint, tab: BodyTab, body: BodySpec): void {
  draftsOf(endpoint).slots[tab] = body
}

/** 该页签在没归档过 Content-Type 时的默认 MIME（null = 不该有这个头）。 */
function defaultContentType(tab: BodyTab, rawSubtype: RawSubtype): string | null {
  switch (tab) {
    case 'none':
    case 'form-data':
      // 空体无 MIME；multipart 的 boundary 由执行器生成
      return null
    case 'x-www-form-urlencoded':
      return 'application/x-www-form-urlencoded'
    case 'binary':
      return 'application/octet-stream'
    case 'graphql':
      return 'application/json'
    case 'raw':
      return rawMime(rawSubtype)
  }
}

/** 归档某页签当前的 Content-Type（离开该页签、或手动改了子类型/请求头后调用）。 */
export function rememberTabContentType(endpoint: Endpoint, tab: BodyTab, headers: KeyValue[]): void {
  draftsOf(endpoint).contentTypes[tab] = contentTypeOf(headers) || null
}

/**
 * 还原某页签自己的 Content-Type：有归档就原样写回（cURL 导入的
 * `application/x-www-form-urlencoded` 等用户真实请求头不被模式默认值覆盖），
 * 没归档才用该模式的默认 MIME。
 */
export function restoreTabContentType(endpoint: Endpoint, tab: BodyTab, headers: KeyValue[]): void {
  const source = draftsOf(endpoint)
  const value = source.contentTypes[tab] || defaultContentType(tab, source.rawSubtype)
  if (value) syncContentType(headers, value)
  else removeContentType(headers)
  source.contentTypes[tab] = value
}

/** 单测隔离用：清空全部草稿（模块作用域在用例之间存活）。 */
export function resetBodyDrafts(): void {
  drafts.clear()
}
