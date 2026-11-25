/**
 * 调用参数骨架：把「每次都点编辑 JSON 手写结构」变成「直接改需要的那几个键」。
 *
 * 背景（真实反馈，2026-11）：attach 调试里集合 / Map / DTO 参数的默认值一律是
 * `null`，用户每次调用都要点开「编辑 JSON」、手写完整对象，调一次改十次。
 *
 * 这里只做**纯函数**（无 Vue / 无 store 依赖），方便单测覆盖：
 * - `defaultArgFor`：按形参类型给出最小可执行骨架；
 * - `dtoSkeleton`：由类名推断常见 DTO 的键（无反射，不猜业务字段）；
 * - `mergeArgs`：把上次调用的参数按位置合并到新骨架（类型不匹配就丢弃）。
 *
 * 原则：宁可给 `null` 也不猜业务字段。骨架只覆盖「结构必然如此」的情形
 *（集合 → `[]`、Map → `{}`、数组 → `[]`）。
 */

/** 整数族（JSON 里都是 number）。 */
const INTEGRAL = new Set([
  'int', 'Integer', 'short', 'Short', 'byte', 'Byte', 'long', 'Long',
  'BigInteger',
])

const DECIMAL = new Set(['float', 'Float', 'double', 'Double', 'BigDecimal'])

const LISTS = new Set([
  'List', 'ArrayList', 'LinkedList', 'Collection', 'Iterable',
  'Set', 'HashSet', 'LinkedHashSet', 'TreeSet',
])

const MAPS = new Set([
  'Map', 'HashMap', 'LinkedHashMap', 'TreeMap', 'JSONObject',
])

/**
 * 常见 DTO 的键骨架（按类名后缀 / 精确名匹配）。
 *
 * **不**按字符串猜业务字段：只覆盖名字本身就说明结构的类型
 *（分页请求、包装响应、时间区间等）。其余返回 null。
 */
const DTO_TEMPLATES: Record<string, Record<string, unknown>> = {
  // 分页请求
  PageQuery: { pageNo: 1, pageSize: 10 },
  PageRequest: { page: 0, size: 20 },
  Pageable: { page: 0, size: 20 },
  // 时间区间
  DateRange: { startDate: null, endDate: null },
  TimeRange: { startTime: null, endTime: null },
  // 金额
  Money: { amount: 0, currency: 'CNY' },
}

/** 数组类型（`Foo[]`）→ 空数组骨架。 */
function isArrayType(name: string): boolean {
  return /\[\]$/.test(name)
}

/**
 * 泛型包装（Optional<T> 等）→ 内层类型。
 *
 * 需先忽略可能的类型实参列表（agent 侧 `getTypeName()` 可能给
 * `java.util.Optional<T>` 之外的形态），并兼容 `Optional<Map<..>>` 的嵌套括号。
 */
function unwrapGeneric(name: string): string | null {
  const trimmed = name.trim()
  // 只看简单类型名（`Optional<...>`）；`.+` 贪婪匹配会跨过嵌套 `>`，故手动配对括号
  const head = trimmed.split('<')[0]
  const shortName = head.split('.').pop() ?? ''
  if (shortName !== 'Optional' && shortName !== 'CompletableFuture' && shortName !== 'Future') {
    return null
  }
  const open = trimmed.indexOf('<')
  if (open < 0) return null
  // 由末尾的 `>` 对齐内层尖括号，剥掉外层泛型
  let depth = 0
  let close = -1
  for (let i = open; i < trimmed.length; i++) {
    if (trimmed[i] === '<') depth++
    else if (trimmed[i] === '>') {
      depth--
      if (depth === 0) {
        close = i
        break
      }
    }
  }
  if (close < 0) return null
  const inner = trimmed.slice(open + 1, close).trim().replace(/^\?\s*(?:extends|super)\s+/, '')
  return inner && inner !== trimmed ? inner : null
}

/** 简单类型名（剥包名，保留数组后缀与泛型）。 */
export function simpleTypeName(typeName: string | null | undefined): string {
  if (!typeName) return ''
  const raw = typeName.trim()
  if (!raw) return ''
  // `java.util.List<com.x.Foo>` → `List<com.x.Foo>`
  const genericStart = raw.indexOf('<')
  if (genericStart >= 0) {
    const head = raw.slice(0, genericStart)
    return `${head.split('.').pop() ?? head}${raw.slice(genericStart)}`
  }
  // `com.x.Foo[]` → `Foo[]`
  if (isArrayType(raw)) {
    return `${raw.slice(0, -2).split('.').pop() ?? ''}[]`
  }
  return raw.split('.').pop() ?? raw
}

/**
 * 常见 DTO 的空骨架；不认识的类返回 null（宁缺勿猜）。
 *
 * @param typeName 形参类型（可带包名 / 泛型）
 */
export function dtoSkeleton(typeName: string | null | undefined): Record<string, unknown> | null {
  const simple = simpleTypeName(typeName)
  if (!simple) return null
  // 剥泛型后再匹配：`PageQuery<T>` → `PageQuery`
  const bare = simple.split('<')[0]
  const hit = DTO_TEMPLATES[bare]
  if (hit) return { ...hit }
  // 通用包装响应：`Result` / `R` / `Response` / `ApiResult`
  if (/^(Result|R|Resp|Response|ApiResult|BaseResult)$/.test(bare)) {
    return { code: 0, message: '', data: null }
  }
  return null
}

/**
 * 按形参类型给出默认值。
 *
 * @param typeName    擦除类型（如 `java.util.List`）
 * @param genericType 泛型签名（如 `java.util.List<com.x.Foo>`），优先用于推断
 */
export function defaultArgFor(
  typeName: string | null | undefined,
  genericType?: string | null,
): unknown {
  const generic = (genericType ?? '').trim()
  const type = simpleTypeName(typeName)
  const byGeneric = simpleTypeName(generic)

  // 以泛型签名优先（能区分 List<Foo> 与 List）
  const probe = byGeneric || type
  if (!probe) return null

  // Optional<T>：递归内层（Optional 本身序列化不了，调用方通常传内层）
  const inner = unwrapGeneric(probe)
  if (inner) return defaultArgFor(inner)

  if (isArrayType(probe)) return []

  // 泛型签名里的外层类型（`Map<String, X>` 的 `Map`）：集合 / Map 判定只看外层，
  // 否则带类型实参的签名永远落不到 MAPS / LISTS 分支
  const outer = probe.split('<')[0]

  switch (outer) {
    case 'String':
    case 'char':
    case 'Character':
    case 'CharSequence':
      return ''
    case 'boolean':
    case 'Boolean':
      return false
    default:
      break
  }
  if (INTEGRAL.has(outer) || DECIMAL.has(outer)) return 0
  if (LISTS.has(outer)) return []
  if (MAPS.has(outer)) return {}

  // 兜底再试擦除类型（泛型可能是裸类型声明）
  if (probe !== type && type) {
    if (LISTS.has(type)) return []
    if (MAPS.has(type)) return {}
    if (INTEGRAL.has(type) || DECIMAL.has(type)) return 0
  }

  return dtoSkeleton(probe) ?? dtoSkeleton(type)
}

/**
 * 粗粒度类型兼容（避免把上次的 `"abc"` 塞进 `long` 形参）。
 *
 * 骨架是**权威类型来源**：骨架说这里是字符串，上次的数字就必须丢弃。
 * 只允许「骨架空位（null / ''）被上次值接管」这一种放松，且仅当值类型
 * 与骨架其它位不冲突（由 `isEmptySkeleton` 分支控制）。
 */
function compatible(previous: unknown, fallback: unknown): boolean {
  if (previous == null) return false
  // 骨架为 null：类型未知，允许沿用（未知结构不该重置用户输入）
  if (fallback === null) return true
  if (typeof fallback === 'number') return typeof previous === 'number'
  if (typeof fallback === 'boolean') return typeof previous === 'boolean'
  if (typeof fallback === 'string') return typeof previous === 'string'
  if (Array.isArray(fallback)) return Array.isArray(previous)
  if (typeof fallback === 'object') {
    return previous != null && typeof previous === 'object' && !Array.isArray(previous)
  }
  return false
}

/**
 * 把上次调用的参数按位置合并到新骨架。
 *
 * 规则：类型兼容就沿用上次值（用户改一次就能复用），不兼容就保留骨架默认。
 * 这是「重跑同一个方法不用重填」的核心。
 */
export function mergeArgs(previous: unknown[] | null | undefined, skeleton: unknown[]): unknown[] {
  if (!previous || previous.length !== skeleton.length) return skeleton
  return skeleton.map((fallback, i) => {
    const prev = previous[i]
    return compatible(prev, fallback) ? prev : fallback
  })
}