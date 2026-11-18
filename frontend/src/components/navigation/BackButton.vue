<script setup lang="ts">
/**
 * BackButton：全局统一返回按钮（导航规范第三/五阶段）。
 *
 * 规格（全局唯一实现，页面不得自行拼返回钮）：
 * - 高度 28px（--h-sm，与 miyo-btn-sm / IconButton 同阶梯），图标 chevron-left 14px；
 * - 带文字时 --fs-xs + gap 4px + 左右 10px 内距；纯图标时 28×28；
 * - 中性常态（无底色 text-2）→ hover 微底色（--bg-hover）→ active 按压（--bg-active）；
 *   focus-visible 主题色描边；title 原生 Tooltip；
 * - 视觉层级低于页面标题：不描边、不填充、不上主题色。
 *
 * 行为（第四阶段返回策略，经 useGoBack 单点实现）：
 * - 默认：会话内有历史 → router.back()；深链直开 → fallback（默认 /projects）；
 * - `onBack` 守卫（离开确认钩子）：返回 false 拦截本次返回（如脏数据确认弹窗
 *   后由页面自行决定去向），其余放行默认返回。
 */
import { computed } from 'vue'
import Icon from '@/components/ui/Icon.vue'
import { useLocaleStore } from '@/stores/locale'
import { useGoBack, NAV_HOME } from '@/composables/useGoBack'

const props = withDefaults(
  defineProps<{
    /** 按钮文字（如「返回」）。缺省渲染纯图标箭头（工作台顶栏紧凑场景）。 */
    label?: string
    /** Tooltip 文案。缺省：有文字→「返回上一页」，纯图标→「返回」。 */
    title?: string
    /** 深链直开（无可回退历史）时的兜底目标。 */
    fallback?: string
    /** 返回守卫：返回 false 拦截默认返回（页面自行决定去向，如先弹脏确认）。 */
    onBack?: () => boolean | void
  }>(),
  { label: '', title: '', fallback: NAV_HOME },
)

const emit = defineEmits<{ click: [event: MouseEvent] }>()

const t = useLocaleStore().t
const goBack = useGoBack()

/** Tooltip 缺省值：纯图标按钮需要名字才可发现，带文字按钮提示「去哪里」。 */
const resolvedTitle = computed(() => props.title || (props.label ? t('nav.backHint') : t('nav.back')))

function onClick(event: MouseEvent): void {
  emit('click', event)
  // 守卫返回 false = 拦截（页面已接管去向，如脏确认后手动导航）；其余放行默认。
  if (props.onBack && props.onBack() === false) return
  goBack(props.fallback)
}
</script>

<template>
  <button type="button" class="back-btn" :class="{ 'with-label': !!label }" :title="resolvedTitle" @click="onClick">
    <Icon name="chevron-left" :size="14" :stroke-width="1.5" />
    <span v-if="label" class="back-btn-label">{{ label }}</span>
  </button>
</template>

<style scoped>
.back-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 4px;
  height: var(--h-sm);
  min-width: var(--h-sm);
  flex-shrink: 0;
  padding: 0 6px;
  border: none;
  background: transparent;
  color: var(--text-2);
  border-radius: var(--radius);
  font-family: inherit;
  cursor: pointer;
  transition:
    background var(--dur) var(--ease),
    color var(--dur) var(--ease),
    transform var(--dur) var(--ease);
}
.back-btn.with-label {
  padding: 0 10px;
}
.back-btn:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}
.back-btn:active {
  background: var(--bg-active);
  transform: scale(0.96);
}
.back-btn:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 1px;
}
.back-btn-label {
  font-size: var(--fs-xs);
  line-height: 1;
  white-space: nowrap;
}
</style>
