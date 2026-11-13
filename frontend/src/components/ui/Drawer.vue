<script setup lang="ts">
/**
 * Drawer：右侧侧滑抽屉。
 * - v-model:open 受控；Esc / 遮罩点击 / 右上 ✕ 关闭；
 * - 打开后焦点移入抽屉、Tab 圈定在内部（焦点陷阱），关闭后归还触发元素；
 * - 打开期间锁定 body 滚动；Teleport 到 body。
 */
import { onBeforeUnmount, ref, watch } from 'vue'
import { useLocaleStore } from '@/stores/locale'
import { pushEscHandler } from '@/utils/escStack'
import IconButton from '@/components/ui/IconButton.vue'

const props = withDefaults(
  defineProps<{
    open: boolean
    title?: string
    width?: string
    closable?: boolean
  }>(),
  { title: '', width: '640px', closable: true },
)

const emit = defineEmits<{
  'update:open': [open: boolean]
  close: []
}>()

const bodyEl = ref<HTMLElement | null>(null)

const titleId = `miyo-drawer-title-${Math.random().toString(36).slice(2, 9)}`

const locale = useLocaleStore()
const t = locale.t

function close(): void {
  emit('update:open', false)
  emit('close')
}

function focusables(): HTMLElement[] {
  if (!bodyEl.value) return []
  return Array.from(
    bodyEl.value.querySelectorAll<HTMLElement>(
      'a[href], button:not([disabled]), textarea:not([disabled]), input:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])',
    ),
  )
}

function onKeydown(event: KeyboardEvent): void {
  // ESC 交给 escStack 层级仲裁（子弹层打开时后开先关）；这里只承担 Tab 焦点陷阱。
  if (event.key !== 'Tab') return
  const root = bodyEl.value
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
    if (!inside || active === first) {
      event.preventDefault()
      last.focus()
    }
  } else if (!inside || active === last) {
    event.preventDefault()
    first.focus()
  }
}

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
        close()
        return true
      })
      // 首个可聚焦元素可能随内容异步渲染，先兜底聚焦容器
      setTimeout(() => {
        ;(focusables()[0] ?? bodyEl.value)?.focus()
      }, 0)
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

onBeforeUnmount(() => {
  document.body.style.overflow = ''
  document.removeEventListener('keydown', onKeydown)
  offEsc?.()
  offEsc = null
})
</script>

<template>
  <Teleport to="body">
    <Transition name="d">
      <div v-if="open" class="d-mask" @mousedown.self="closable && close()">
        <aside
          ref="bodyEl"
          class="d-drawer"
          role="dialog"
          aria-modal="true"
          :aria-labelledby="title ? titleId : undefined"
          tabindex="-1"
          :style="{ width }"
          @mousedown.stop
        >
          <div class="d-head">
            <h3 :id="titleId" class="d-title">{{ title }}</h3>
            <IconButton v-if="closable" name="x" :size="14" :title="t('common.close')" @click="close" />
          </div>
          <div class="d-body">
            <slot />
          </div>
        </aside>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.d-mask {
  position: fixed;
  inset: 0;
  z-index: var(--z-modal);
  display: flex;
  justify-content: flex-end;
  background: var(--mask);
}

.d-drawer {
  display: flex;
  flex-direction: column;
  height: 100%;
  max-width: 96vw;
  border-left: 1px solid var(--border);
  border-radius: var(--radius-lg) 0 0 var(--radius-lg);
  background: var(--bg-elevated);
  box-shadow: var(--shadow-drawer);
  overflow: hidden;
}

.d-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 14px 16px;
  border-bottom: 1px solid var(--border);
  flex-shrink: 0;
}

.d-title {
  margin: 0;
  font-size: var(--fs-md);
  font-weight: 600;
  color: var(--text-1);
}

.d-body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 16px;
}

.d-enter-active,
.d-leave-active {
  transition: transform 220ms var(--ease), opacity 220ms var(--ease);
}

.d-mask {
  transition: background 220ms var(--ease);
}

.d-enter-from,
.d-leave-to {
  background: transparent;
}

.d-enter-from .d-drawer,
.d-leave-to .d-drawer {
  transform: translateX(100%);
}
</style>