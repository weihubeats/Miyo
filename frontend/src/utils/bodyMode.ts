/**
 * bodyMode：Body 类型 Tab ↔ BodySpec 映射与 Content-Type 联动。
 *
 * UI Tab（none / form-data / x-www-form-urlencoded / raw / binary / graphql）
 * 与后端 BodySpec（none / multipart / urlencoded / json|text / binary / graphql）
 * 不一一对应：raw 是 json+text 的聚合视图，其子类型（JSON/Text/JS/HTML/XML）
 * 不新增存储字段，而是通过 Content-Type 请求头持久化——发送时用户显式头
 * 优先于模式默认 MIME（miyo-http client.rs），因此只需同步头即可生效。
 */
import type { BodySpec, KeyValue, MultipartField, RequestSpec } from '../types/miyoApi'

/** Body 面板顶层 Tab（Postman 风格）。 */
export type BodyTab =
  | 'none'
  | 'form-data'
  | 'x-www-form-urlencoded'
  | 'raw'
  | 'binary'
  | 'graphql'

/** raw 子类型。 */
export type RawSubtype = 'json' | 'text' | 'javascript' | 'html' | 'xml'

export const RAW_SUBTYPES: Array<{ value: RawSubtype; label: string; mime: string }> = [
  { value: 'json', label: 'JSON', mime: 'application/json' },
  { value: 'text', label: 'Text', mime: 'text/plain' },
  { value: 'javascript', label: 'JavaScript', mime: 'text/javascript' },
  { value: 'html', label: 'HTML', mime: 'text/html' },
  { value: 'xml', label: 'XML', mime: 'application/xml' },
]

/** 读取启用中的 Content-Type 头（大小写不敏感）。 */
export function contentTypeOf(headers: KeyValue[]): string {
  for (const h of headers) {
    if (h.enabled && h.key.trim().toLowerCase() === 'content-type') return h.value.trim()
  }
  return ''
}

/** 更新或追加 Content-Type 请求头（原位更新，不动其他头）。 */
export function syncContentType(headers: KeyValue[], mime: string): void {
  const row = headers.find((h) => h.key.trim().toLowerCase() === 'content-type')
  if (row) {
    row.key = 'Content-Type'
    row.value = mime
    row.enabled = true
  } else {
    headers.push({ key: 'Content-Type', value: mime, enabled: true, description: '', field_type: 'string', required: true, example: '' })
  }
}

/** 移除 Content-Type 请求头（form-data 的 boundary、none 空体由执行器按需生成）。 */
export function removeContentType(headers: KeyValue[]): void {
  for (let i = headers.length - 1; i >= 0; i -= 1) {
    if (headers[i].key.trim().toLowerCase() === 'content-type') headers.splice(i, 1)
  }
}

/** 由 BodySpec + 请求头推导当前 Tab。 */
export function tabOf(body: BodySpec, _headers: KeyValue[]): BodyTab {
  switch (body.mode) {
    case 'none':
      return 'none'
    case 'multipart':
      return 'form-data'
    case 'urlencoded':
      return 'x-www-form-urlencoded'
    case 'binary':
      return 'binary'
    case 'graphql':
      return 'graphql'
    case 'json':
    case 'text':
      return 'raw'
    // gRPC 配置走独立 GrpcPanel 页签，不经由 Body 页签推导
    case 'grpc':
      return 'none'
  }
}

/** raw 子类型推导：json 模式恒为 JSON；text 模式按 Content-Type 头判断。 */
export function rawSubtypeOf(body: BodySpec, headers: KeyValue[]): RawSubtype {
  if (body.mode === 'json') return 'json'
  const ct = contentTypeOf(headers).toLowerCase()
  if (ct.includes('html')) return 'html'
  if (ct.includes('xml')) return 'xml'
  if (ct.includes('javascript') || ct.includes('ecmascript')) return 'javascript'
  return 'text'
}

/** 切换 raw 子类型：JSON → json 模式，其余 → text 模式；raw 文本跨子类型保留。 */
export function applyRawSubtype(req: RequestSpec, subtype: RawSubtype): void {
  const prev = req.body
  const raw = prev.mode === 'json' || prev.mode === 'text' ? prev.raw : ''
  restoreRaw(req, subtype, raw)
}

/** raw 子类型对应的 MIME。 */
export function rawMime(subtype: RawSubtype): string {
  return RAW_SUBTYPES.find((s) => s.value === subtype)?.mime ?? 'text/plain'
}

/**
 * 还原 raw（子类型 + 文本显式指定）：切到 none 会整体替换 body 并移除
 * Content-Type，切回 raw 时仅靠推导只能默认 text——调用方传入离开前
 * 记忆的子类型与文本，还原用户之前的选择。
 */
export function restoreRaw(req: RequestSpec, subtype: RawSubtype, raw: string): void {
  req.body = subtype === 'json' ? { mode: 'json', raw } : { mode: 'text', raw }
  syncContentType(req.headers, rawMime(subtype))
}

/** KeyValue（urlencoded 行）→ MultipartField（文本 part）。 */
function toMultipartField(kv: KeyValue): MultipartField {
  return { key: kv.key, value_type: 'text', value: kv.value, enabled: kv.enabled }
}

/** MultipartField → KeyValue（文本 part 退化为 urlencoded 行；文件路径丢弃）。 */
function toKeyValue(f: MultipartField): KeyValue {
  return { key: f.key, value: f.value_type === 'text' ? f.value : '', enabled: f.enabled, description: '', field_type: 'string', required: true, example: '' }
}

/**
 * 切换 Body Tab：整体替换 body 形状（避免残留多余字段），并同步 Content-Type：
 * - form-data / none：移除显式头（boundary 由执行器生成；none 无 body）；
 * - 其余模式：固定 MIME（残留旧值会因「用户头优先」破坏请求）。
 */
export function applyBodyTab(req: RequestSpec, tab: BodyTab): void {
  const prev = req.body
  switch (tab) {
    case 'none':
      req.body = { mode: 'none' }
      removeContentType(req.headers)
      break
    case 'form-data':
      req.body = {
        mode: 'multipart',
        fields: prev.mode === 'urlencoded' ? prev.fields.map(toMultipartField) : prev.mode === 'multipart' ? prev.fields : [],
      }
      removeContentType(req.headers)
      break
    case 'x-www-form-urlencoded':
      req.body = {
        mode: 'urlencoded',
        fields: prev.mode === 'multipart' ? prev.fields.map(toKeyValue) : prev.mode === 'urlencoded' ? prev.fields : [],
      }
      syncContentType(req.headers, 'application/x-www-form-urlencoded')
      break
    case 'binary':
      req.body = { mode: 'binary', path: prev.mode === 'binary' ? prev.path : '' }
      syncContentType(req.headers, 'application/octet-stream')
      break
    case 'graphql':
      req.body = {
        mode: 'graphql',
        spec:
          prev.mode === 'graphql'
            ? prev.spec
            : { query: '', variables: '{}', operation_name: '' },
      }
      syncContentType(req.headers, 'application/json')
      break
    case 'raw':
      applyRawSubtype(req, rawSubtypeOf(prev, req.headers))
      break
  }
}
