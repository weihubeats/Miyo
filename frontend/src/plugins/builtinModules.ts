/**
 * builtin 模块表：内置插件 ID → 仓内插件入口的懒加载工厂。
 *
 * 关键约束：**未启用的插件绝不 import**——模块代码只在 activate 时经动态
 * import 拉取独立 chunk（含其视图组件），未安装 / 停用的插件零字节加载。
 * 新增内置插件：在 miyo-plugin/builtins/<id>/manifest.json 落清单（后端目录
 * 是唯一事实源），再在这里补一行映射。
 */
import type { BuiltinModuleMap } from './types'
import { JAVA_DEBUG_PLUGIN_ID } from './builtinIds'

export const builtinModules: BuiltinModuleMap = {
  [JAVA_DEBUG_PLUGIN_ID]: () => import('@/domains/java-debug/plugin'),
}
