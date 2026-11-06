/**
 * methodTone：HTTP 方法语义色（@theme 方法色工具类）。
 *
 * - 背景：方法色曾以 rgba 硬编码散落在各组件（树徽章/标签页/文本），深浅主题
 *   各自为政；收敛到 @theme 的 method-* 令牌后，色调只在此一处声明，
 *   深/浅主题自动跟随方法变量（--get/--post/--put/--patch/--delete）。
 * - 注意：Tailwind 扫描的是源码字面量，此处完整类名常量可被正常收录；
 *   不要拼接构造类名（如 `text-method-${x}`），否则 utilities 不会生成。
 */

/** 徽章式：文字 + 10% 底 + 20% 描边（树 / 标签页胶囊）。 */
export const METHOD_TONE: Record<string, string> = {
  get: 'text-method-get bg-method-get/10 border-method-get/20',
  post: 'text-method-post bg-method-post/10 border-method-post/20',
  put: 'text-method-put bg-method-put/10 border-method-put/20',
  delete: 'text-method-delete bg-method-delete/10 border-method-delete/20',
  patch: 'text-method-patch bg-method-patch/10 border-method-patch/20',
  graphql: 'text-method-patch bg-method-patch/10 border-method-patch/20',
  grpc: 'text-method-grpc bg-method-grpc/10 border-method-grpc/20',
  head: 'text-method-neutral bg-method-neutral/10 border-method-neutral/20',
  options: 'text-method-neutral bg-method-neutral/10 border-method-neutral/20',
}

/** 徽章式（主操作栏 Method 选择器，与侧栏标签块一致）：15% 淡底 + 方法色文字。 */
export const METHOD_BADGE_TONE: Record<string, string> = {
  get: 'bg-method-get/15 text-method-get',
  post: 'bg-method-post/15 text-method-post',
  put: 'bg-method-put/15 text-method-put',
  delete: 'bg-method-delete/15 text-method-delete',
  patch: 'bg-method-patch/15 text-method-patch',
  graphql: 'bg-method-patch/15 text-method-patch',
  grpc: 'bg-method-grpc/15 text-method-grpc',
  head: 'bg-method-neutral/15 text-method-neutral',
  options: 'bg-method-neutral/15 text-method-neutral',
}

/** 纯文本式：仅文字色（TestCaseDrawer 徽章底 / 历史行等用 currentColor 派生底色处）。 */
export const METHOD_TEXT_TONE: Record<string, string> = {
  get: 'text-method-get',
  post: 'text-method-post',
  put: 'text-method-put',
  delete: 'text-method-delete',
  patch: 'text-method-patch',
  graphql: 'text-method-patch',
  grpc: 'text-method-grpc',
  head: 'text-method-neutral',
  options: 'text-method-neutral',
}

function toneOf(map: Record<string, string>, method: string): string {
  return map[method.toLowerCase()] ?? map.options
}

/** 徽章式语义类（未知方法兜底中性灰）。 */
export function methodTone(method: string): string {
  return toneOf(METHOD_TONE, method)
}

/** 徽章式语义类（未知方法兜底中性灰）。 */
export function methodBadgeTone(method: string): string {
  return toneOf(METHOD_BADGE_TONE, method)
}

/** 纯文本式语义类（未知方法兜底中性灰）。 */
export function methodTextTone(method: string): string {
  return toneOf(METHOD_TEXT_TONE, method)
}

