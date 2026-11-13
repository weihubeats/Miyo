<script setup lang="ts">
/**
 * Modal：居中模态弹层。
 * - v-model:open 受控；Esc / 遮罩点击 / 右上 ✕ 关闭（可经 guardClose 拦截）；
 * - 打开后自动聚焦首个可聚焦元素（autofocus 默认开启），关闭后焦点归还触发元素；
 * - Tab 焦点陷阱：焦点始终圈定在对话框内，不穿透遮罩到背景内容；
 * - 打开期间锁定 body 滚动；Teleport 到 body，双主题。
 */
import { nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { useLocaleStore } from '@/stores/locale'
import { pushEscHandler } from '@/utils/escStack'
import IconButton from '@/components/ui/IconButton.vue'

const props = withDefaults(
  defineProps<{
    open: boolean
    title?: string
    width?: string
    closable?: boolean
    autofocus?: boolean
    /** 附加到对话框容器的类名（可经 CSS 变量覆盖底色/边框/圆角/阴影，用于高级定制外观）。 */
    dialogClass?: string
    /** 附加到遮罩层的类名（如高斯模糊遮罩），默认无。 */
    maskClass?: string
    /** 关闭前守卫：返回 false 拦截 Esc / 遮罩 / ✕ 触发的关闭（用于脏数据确认）。 */
    guardClose?: () => boolean
  }>(),
  {
    title: '',
    width: '420px',
    closable: true,
    autofocus: true,
    dialogClass: '',
    maskClass: '',
    guardClose: undefined,
  },
)

const emit = defineEmits<{
  'update:open': [open: boolean]
  close: []
}>()

const titleId = `miyo-modal-title-${Math.random().toString(36).slice(2, 9)}`

const locale = useLocaleStore()
const t = locale.t

const dialogEl = ref<HTMLElement | null>(null)

/** 关闭请求统一入口：守卫放行才真正关闭。 */
function requestClose(): void {
  if (props.guardClose && !props.guardClose()) return
  emit('update:open', false)
  emit('close')
}

/** 对话框内可聚焦元素（Tab 陷阱的目标集合）。 */
function focusables(): HTMLElement[] {
  if (!dialogEl.value) return []
  return Array.from(
    dialogEl.value.querySelectorAll<HTMLElement>(
      'a[href], button:not([disabled]), textarea:not([disabled]), input:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])',
    ),
  )
}

function onKeydown(event: KeyboardEvent): void {
  // ESC 交给 escStack 层级仲裁（子弹层打开时后开先关，不连关本 Modal）；
  // 这里只承担 Tab 焦点陷阱。
  if (event.key !== 'Tab') return
  const root = dialogEl.value
  if (!root) return
  const items = focusables()
  if (!items.length) {
    event.preventDefault()
    root.focus()
    return
  }
  const first = items[0]
  const last = items[items.length - 1]
  const active = document.activeElement
  const inside = !!active && root.contains(active)
  if (event.shiftKey) {
    // 逆序走到头（或焦点在对话框外）→ 回到最后一个
    if (!inside || active === first) {
      event.preventDefault()
      last.focus()
    }
  } else if (!inside || active === last) {
    event.preventDefault()
    first.focus()
  }
}

/** 关闭时把焦点还给触发元素，避免屏幕阅读器/键盘用户丢失位置。 */
let lastFocused: HTMLElement | null = null
/** escStack 卸载函数（打开期间持有）。 */
let offEsc: (() => void) | null = null

watch(
  () => props.open,
  (open) => {
    if (open) {
      lastFocused = document.activeElement as HTMLElement | null
      document.body.style.overflow = 'hidden'
      document.addEventListener('keydown', onKeydown)
      offEsc = pushEscHandler(() => {
        requestClose()
        return true
      })
      if (props.autofocus) {
        nextTick(() => {
          focusables()[0]?.focus()
        })
      }
    } else {
      document.body.style.overflow = ''
      document.removeEventListener('keydown', onKeydown)
      offEsc?.()
      offEsc = null
      lastFocused?.focus?.()
      lastFocused = null
    }
  },
)

// 打开中卸载兜底：原来只在 watch(open=false) 时清理，打开状态下直接卸载
// 会泄漏 document keydown 监听 + body 锁定（Drawer.vue 已有正确示范）。
onBeforeUnmount(() => {
  document.removeEventListener('keydown', onKeydown)
  offEsc?.()
  offEsc = null
  if (props.open) document.body.style.overflow = ''
})
</script>

<template>
  <Teleport to="body">
    <Transition name="m">
      <div v-if="open" class="m-mask" :class="maskClass" @mousedown.self="closable && requestClose()">
        <div
          ref="dialogEl"
          class="m-dialog"
          :class="dialogClass"
          role="dialog"
          aria-modal="true"
          :aria-labelledby="title ? titleId : undefined"
          tabindex="-1"
          :style="{ width }"
          @mousedown.stop
        >
          <div v-if="title" class="m-head">
            <h3 :id="titleId" class="m-title">{{ title }}</h3>
            <IconButton v-if="closable" name="x" :size="14" :title="t('common.close')" @click="requestClose" />
          </div>
          <div class="m-body">
            <slot />
          </div>
          <div v-if="$slots.footer" class="m-foot">
            <slot name="footer" />
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.m-mask {
  position: fixed;
  inset: 0;
  z-index: var(--z-modal);
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
  background: var(--mask);
}

.m-dialog {
  display: flex;
  flex-direction: column;
  max-width: 92vw;
  max-height: 84vh;
  border-radius: var(--radius-lg);
  border: 1px solid var(--border-strong);
  background: var(--bg-elevated);
  box-shadow: var(--shadow-lg);
  overflow: hidden;
  outline: none;
}

.m-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 14px 16px 0;
}

.m-title {
  margin: 0;
  font-size: var(--fs-lg);
  font-weight: 600;
  color: var(--text-1);
}

.m-body {
  padding: 12px 16px 16px;
  overflow-y: auto;
  color: var(--text-2);
  font-size: 13px;
}

.m-foot {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  /* 底部操作栏：hairline 分隔（浅色 = slate-200，深色 = 深灰线），双主题自适配 */
  border-top: 1px solid var(--border);
  padding: 12px 16px 14px;
}

/* 进 200ms / 出 160ms（动画规范：弹窗进 180~220ms、出 150~180ms）；transform 仅轻微位移 */
.m-enter-active,
.m-leave-active {
  transition: opacity 200ms var(--ease);
}
.m-leave-active {
  transition-duration: 160ms;
}
.m-enter-active .m-dialog,
.m-leave-active .m-dialog {
  transition: transform 200ms var(--ease);
}
.m-leave-active .m-dialog {
  transition-duration: 160ms;
}
.m-enter-from,
.m-leave-to {
  opacity: 0;
}
.m-enter-from .m-dialog,
.m-leave-to .m-dialog {
  transform: translateY(-10px) scale(0.98);
}
</style>