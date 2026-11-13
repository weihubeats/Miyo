/**
 * jsonTree：设计页结构化 Body 编辑器的纯函数层。
 *
 * - JSON 文本 ↔ 节点树互转（对象 / 数组 / 标量）；
 * - 文档 sidecar 键为 RFC6901 JSON Pointer（数组元素固定 `/0` 样本路径）；
 * - 树节点自带 description / required，序列化时 flatten 成 `Record<pointer, FieldDoc>`；
 * - 发送 / Mock 忽略 docs，仅设计页消费。
 */
import type { FieldDoc } from '@/types/miyoApi'

export type JsonTreeType = 'string' | 'number' | 'boolean' | 'object' | 'array' | 'null'

export interface JsonTreeNode {
  /** 节点稳定 id（编辑器 v-for key；改名/指针变化不重挂 DOM）。 */
  nid: number
  /** 字段名（根级数组 / 标量为空串）。 */
  key: string
  type: JsonTreeType
  /** 叶子原始文本（string 去引号；number/boolean/null 为字面量）。 */
  value: string
  /** object 子字段 / array 对象元素字段。 */
  children: JsonTreeNode[]
  /** RFC6901 JSON Pointer（相对文档根；数组元素子字段含 `/0`）。 */
  pointer: string
  description: string
  required: boolean
}

/** RFC6901：`~` → `~0`，`/` → `~1`。 */
export function pointerEscape(token: string): string {
  return token.replace(/~/g, '~0').replace(/\//g, '~1')
}

let nidSeq = 0

function nextNid(): number {
  nidSeq += 1
  return nidSeq
}

/** 新节点稳定 id（编辑器新建叶子等场景）。 */
export function freshNid(): number {
  return nextNid()
}

function typeOf(value: unknown): JsonTreeType {
  if (value === null) return 'null'
  if (Array.isArray(value)) return 'array'
  if (typeof value === 'object') return 'object'
  if (typeof value === 'number') return 'number'
  if (typeof value === 'boolean') return 'boolean'
  return 'string'
}

function leafText(type: JsonTreeType, value: unknown): string {
  if (type === 'string') return typeof value === 'string' ? value : ''
  if (type === 'null') return 'null'
  return String(value)
}

function emptyNode(key: string, type: JsonTreeType, pointer: string): JsonTreeNode {
  return {
    nid: nextNid(),
    key,
    type,
    value:
      type === 'string'
        ? ''
        : type === 'number'
          ? '0'
          : type === 'boolean'
            ? 'false'
            : type === 'null'
              ? 'null'
              : '',
    children: [],
    pointer,
    description: '',
    required: true,
  }
}

/** 数组对象元素的子字段指针：`{arrayPtr}/0/{key}`（RFC6901 样本路径约定）。 */
function arrayItemPointer(arrayPtr: string, token: string): string {
  return `${arrayPtr}/0/${pointerEscape(token)}`
}

function buildNode(key: string, value: unknown, pointer: string): JsonTreeNode {
  const type = typeOf(value)
  if (type === 'object') {
    const children = Object.entries(value as Record<string, unknown>).map(([k, v]) =>
      buildNode(k, v, `${pointer}/${pointerEscape(k)}`),
    )
    return { nid: nextNid(), key, type, value: '', children, pointer, description: '', required: true }
  }
  if (type === 'array') {
    const arr = value as unknown[]
    const first = arr.find((v) => v !== null)
    const firstType = first === undefined ? null : typeOf(first)
    const isObjectSample = firstType === 'object' && first !== undefined
    const children = isObjectSample
      ? Object.entries(first as Record<string, unknown>).map(([k, v]) =>
          buildNode(k, v, arrayItemPointer(pointer, k)),
        )
      : []
    // 非对象样本数组（含空数组 / 标量数组）：整段 JSON 存 value 保 roundtrip
    const stored = isObjectSample ? '' : JSON.stringify(arr)
    return { nid: nextNid(), key, type, value: stored, children, pointer, description: '', required: true }
  }
  return {
    nid: nextNid(),
    key,
    type,
    value: leafText(type, value),
    children: [],
    pointer,
    description: '',
    required: true,
  }
}

/**
 * JSON 文本 → 根节点。
 * 空文本 / 空白 → 空对象根；解析失败返回 null（调用方回退文本模式）。
 */
export function parseJsonTree(text: string): JsonTreeNode | null {
  const trimmed = text.trim()
  if (!trimmed) return emptyNode('', 'object', '')
  let parsed: unknown
  try {
    parsed = JSON.parse(trimmed)
  } catch {
    return null
  }
  if (typeOf(parsed) !== 'object') return null
  return buildNode('', parsed, '')
}

function scalarFromNode(node: JsonTreeNode): unknown {
  if (node.type === 'number') {
    const n = Number(node.value.trim())
    return Number.isFinite(n) ? n : 0
  }
  if (node.type === 'boolean') return node.value.trim() === 'true'
  if (node.type === 'null') return null
  return node.value
}

function valueOfNode(node: JsonTreeNode): unknown {
  if (node.type === 'object') {
    const out: Record<string, unknown> = {}
    for (const child of node.children) {
      const k = child.key.trim()
      if (!k || k in out) continue
      out[k] = valueOfNode(child)
    }
    return out
  }
  if (node.type === 'array') {
    if (node.children.length) return [valueOfObjectChildren(node.children)]
    if (node.value.trim()) {
      try {
        const parsed: unknown = JSON.parse(node.value)
        return Array.isArray(parsed) ? parsed : []
      } catch {
        return []
      }
    }
    return []
  }
  return scalarFromNode(node)
}

/** 数组单元素样本：子字段（已是 `/0/...` 指针）组装为对象。 */
function valueOfObjectChildren(children: JsonTreeNode[]): Record<string, unknown> {
  const out: Record<string, unknown> = {}
  for (const child of children) {
    const k = child.key.trim()
    if (!k || k in out) continue
    out[k] = valueOfNode(child)
  }
  return out
}

/** 根节点 → 紧凑 JSON 文本（结构编辑器写出前调用；调用方可再格式化）。 */
export function treeToJsonString(root: JsonTreeNode): string {
  return JSON.stringify(valueOfNode(root))
}

/** 根节点 → 带缩进的 JSON 文本（写回草稿 raw）。 */
export function treeToPrettyJson(root: JsonTreeNode): string {
  return JSON.stringify(valueOfNode(root), null, 2)
}

function flattenDocs(node: JsonTreeNode, out: Record<string, FieldDoc>): void {
  // 根节点（pointer === ''）不产生文档键
  if (node.pointer) {
    const desc = node.description.trim()
    if (desc || !node.required) {
      out[node.pointer] = { description: desc, required: node.required }
    }
  }
  for (const child of node.children) flattenDocs(child, out)
}

/** 树 → sidecar `Record<pointer, FieldDoc>`（空描述且必填的节点不落盘）。 */
export function docsFromTree(root: JsonTreeNode): Record<string, FieldDoc> {
  const out: Record<string, FieldDoc> = {}
  flattenDocs(root, out)
  return out
}

function applyDocs(node: JsonTreeNode, docs: Record<string, FieldDoc>): void {
  const doc = node.pointer ? docs[node.pointer] : undefined
  if (doc) {
    node.description = doc.description ?? ''
    node.required = doc.required ?? true
  }
  for (const child of node.children) applyDocs(child, docs)
}

/** 解析 JSON 并注入 sidecar 注释；非法 JSON 返回 null。 */
export function parseJsonTreeWithDocs(
  text: string,
  docs: Record<string, FieldDoc> | undefined,
): JsonTreeNode | null {
  const root = parseJsonTree(text)
  if (!root) return null
  if (docs) applyDocs(root, docs)
  return root
}

/** 空结构根（`{}`），用于新建 / 非法样本兜底。 */
export function emptyJsonObjectNode(): JsonTreeNode {
  return emptyNode('', 'object', '')
}

/** 深拷贝子树并重算全部指针（改名 / 移动后调用）；保留 nid 稳定 key。 */
export function rebindPointers(node: JsonTreeNode, pointer: string): JsonTreeNode {
  const next: JsonTreeNode = { ...node, pointer, children: [] }
  if (next.type === 'array') {
    next.children = node.children.map((c) => rebindPointers(c, arrayItemPointer(pointer, c.key)))
  } else {
    next.children = node.children.map((c) =>
      rebindPointers(c, `${pointer}/${pointerEscape(c.key)}`),
    )
  }
  return next
}
