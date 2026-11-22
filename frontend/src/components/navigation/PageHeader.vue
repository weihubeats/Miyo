<script setup lang="ts">
/**
 * PageHeader：全局统一页面顶栏（导航规范第二/三阶段）。
 *
 * 覆盖两种页面形态（结构同构，实证合并为单一实现避免再漂移——此前 4 份
 * goBack 拷贝 + 6 种返回钮样式正是各自手写顶栏的产物）：
 * - 普通子页面（插件中心等）：Brand 标题 + 返回钮 + 操作区；
 * - 独立调试工作台（实时调试 / GraphQL / 集成测试 / 插件工作台）：同骨架，
 *   中部 slot 放工作台上下文（模式切换 / 连接状态 / 面包屑），不放返回钮
 *   与核心操作（连接、运行、保存）混排。
 *
 * 骨架：[Brand(标题，点击回首页)] [BackButton] [default slot] [spacer] [actions slot]
 * - 整行可拖拽窗口（.miyo-topbar 全局类，style.css 单点定义），交互子元素豁免；
 * - macOS 主窗交通灯占左上 88px（style.css 的 `data-window='main'` 规则预留，
 *   同 WorkspaceView 惯例）；弹出子窗是原生标题栏，不留死区；
 * - hideBack：根页面（无需返回）；hideBrand：Tauri 弹出子窗（无 Brand 语境）；
 * - @back：页面监听时完全接管返回（BackButton 默认策略不再触发），用于
 *   编辑器态先回列表 + 脏确认等分层返回。
 */
import { computed, getCurrentInstance } from 'vue'
import Brand from '@/domains/workspace/components/Brand.vue'
import BackButton from '@/components/navigation/BackButton.vue'
import { useLocaleStore } from '@/stores/locale'
import { NAV_HOME } from '@/composables/useGoBack'

const props = withDefaults(
  defineProps<{
    /** 页面标题（渲染在 Brand 区，点击回项目首页）。 */
    title: string
    /** Brand 副标题（一般不用）。 */
    subtitle?: string
    /** 页面描述（返回钮右侧的次级说明文字，窄宽截断；缺省不渲染）。 */
    description?: string
    /** 隐藏返回钮（根页面）。 */
    hideBack?: boolean
    /** 隐藏 Brand（Tauri 弹出子窗等无 Brand 语境）。 */
    hideBrand?: boolean
    /** 返回钮文字（默认「返回」；传空串渲染纯图标）。 */
    backLabel?: string
    /** 返回钮 Tooltip 覆盖。 */
    backTitle?: string
    /** 深链直开时的兜底返回目标（默认 /projects）。 */
    backFallback?: string
  }>(),
  {
    subtitle: '',
    description: '',
    hideBack: false,
    hideBrand: false,
    backTitle: '',
    backFallback: NAV_HOME,
  },
)

const emit = defineEmits<{
  /** 页面监听即完全接管返回（默认返回策略不再执行）。 */
  back: []
}>()

const t = useLocaleStore().t

/** 返回按钮文字：未传取统一文案「返回」；显式传空串 = 纯图标返回钮。 */
const resolvedBackLabel = computed(() =>
  props.backLabel === undefined ? t('nav.back') : props.backLabel,
)

/**
 * 页面是否监听 @back（vnode.props.onBack 为父级传入的原始监听）。
 * 监听即接管：经 BackButton 的 onBack 守卫转发事件并拦截默认返回——
 * 编辑器态「先回列表 + 脏确认」等分层返回由此实现。
 */
const hasBackListener = computed(
  () => !!(getCurrentInstance()?.vnode.props as Record<string, unknown> | null)?.onBack,
)

/** 接管态的转发：发出 back 事件并返回 false 拦截 BackButton 默认导航。 */
function forwardBack(): boolean {
  emit('back')
  return false
}
</script>

<template>
  <header class="miyo-topbar">
    <Brand v-if="!hideBrand" :title="title" :subtitle="subtitle" class="miyo-topbar-brand" />
    <BackButton
      v-if="!hideBack"
      :label="resolvedBackLabel"
      :title="backTitle"
      :fallback="backFallback"
      :on-back="hasBackListener ? forwardBack : undefined"
    />
    <!-- 页面描述：次级说明文字（与 Brand subtitle 不同，属于页面而非品牌区） -->
    <span v-if="description" class="miyo-topbar-desc" v-tooltip-overflow>{{ description }}</span>
    <!-- 中部上下文：工作台模式切换 / 面包屑 / 页面级输入等 -->
    <slot></slot>
    <div class="miyo-topbar-spacer" aria-hidden="true"></div>
    <!-- 右侧操作区：页面必要操作（选择器 / 状态徽章 / 主按钮） -->
    <slot name="actions"></slot>
  </header>
</template>
