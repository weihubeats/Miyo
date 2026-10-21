/**
 * skills store：Skill 管理页的单一状态源。
 *
 * - 状态以文件系统为准：`refresh()` 每次全量扫描，不做本地缓存推导；
 * - 启停 / 保存后只用后端返回的单条 `SkillSummary` 替换对应行（避免整表闪烁）；
 * - 作用域：`global`（~/.agents/skills）或 `project`（用户选择的项目目录）。
 */
import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { useFoxApi } from '../composables/useFoxApi'
import type { SkillDoc, SkillIndex, SkillInput, SkillScope, SkillSummary } from '../types/foxApi'

export const useSkillsStore = defineStore('skills', () => {
  const api = useFoxApi()

  const scope = ref<SkillScope>('global')
  const projectDir = ref<string | null>(null)
  const index = ref<SkillIndex | null>(null)
  const loading = ref(false)
  const error = ref('')

  const skills = computed(() => index.value?.skills ?? [])
  const tools = computed(() => index.value?.tools ?? [])
  const foreign = computed(() => index.value?.foreign ?? [])
  const sourceRoot = computed(() => index.value?.sourceRoot ?? '')

  /** 当前作用域对应的目录参数（global 传 null）。 */
  function activeDir(): string | null {
    return scope.value === 'project' ? projectDir.value : null
  }

  /** 切换作用域；项目级需要先有目录。 */
  function setScope(next: SkillScope, dir: string | null = null): void {
    scope.value = next
    projectDir.value = next === 'project' ? dir : null
  }

  /** 全量扫描。失败时抛给调用方（视图负责 toast），同时保留 error 供空态展示。 */
  async function refresh(): Promise<void> {
    if (scope.value === 'project' && !projectDir.value) {
      index.value = null
      return
    }
    loading.value = true
    error.value = ''
    try {
      index.value = await api.skillsScan(scope.value, activeDir())
    } catch (e) {
      error.value = e instanceof Error ? e.message : String(e)
      index.value = null
      throw e
    } finally {
      loading.value = false
    }
  }

  /** 用后端返回的单条摘要替换行。 */
  function patch(summary: SkillSummary): void {
    if (!index.value) return
    const at = index.value.skills.findIndex((skill) => skill.name === summary.name)
    if (at >= 0) index.value.skills.splice(at, 1, summary)
    else index.value.skills.push(summary)
  }

  /** 启停单个工具，成功后就地更新该行。 */
  async function setEnabled(name: string, toolId: string, enabled: boolean): Promise<SkillSummary> {
    const summary = await api.skillSetEnabled(name, toolId, enabled, scope.value, activeDir())
    patch(summary)
    return summary
  }

  /** 打开真源目录（opener 在视图层调用，store 只取路径）。 */
  async function dirPath(name: string): Promise<string> {
    return api.skillDirPath(name, scope.value, activeDir())
  }

  /** 读取详情（编辑器用）。 */
  async function read(name: string): Promise<SkillDoc> {
    return api.skillRead(name, scope.value, activeDir())
  }

  /** 新建 skill（只写真源，不自动启用）。 */
  async function create(input: SkillInput): Promise<SkillSummary> {
    const summary = await api.skillCreate(input, scope.value, activeDir())
    if (index.value) {
      index.value.skills.push(summary)
      index.value.skills.sort((a, b) => a.name.localeCompare(b.name))
    }
    return summary
  }

  /** 保存 skill（rawExpected 不一致 → CONFLICT）。 */
  async function save(name: string, input: SkillInput): Promise<SkillSummary> {
    const summary = await api.skillSave(name, input, scope.value, activeDir())
    patch(summary)
    return summary
  }

  /** 删除 skill（真源进 trash），成功后从列表移除。 */
  async function remove(name: string): Promise<void> {
    await api.skillDelete(name, scope.value, activeDir())
    if (index.value) {
      index.value.skills = index.value.skills.filter((skill) => skill.name !== name)
    }
  }

  return {
    scope,
    projectDir,
    index,
    loading,
    error,
    skills,
    tools,
    foreign,
    sourceRoot,
    setScope,
    refresh,
    setEnabled,
    dirPath,
    read,
    create,
    save,
    remove,
    patch,
  }
})
