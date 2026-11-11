<script setup lang="ts">
/**
 * Popconfirm：危险操作确认气泡（替代原生 confirm）。
 * - 触发区为默认插槽；点击展开气泡，外部点击 / Esc 关闭；
 * - 展开监听走**捕获阶段**：插槽里的触发按钮常带 `@click.stop`（行内操作按钮
 *   要阻止触发行点击），冒泡阶段到不了外层 .pc-trigger，气泡就永远弹不出来
 *   （v0.0.54 用户实测「场景点删除没反应」即此因）；
 * - Teleport + 定位测量，底部空间不足自动向上翻转；
 * - 视觉：危险图标徽章 + 标题（可选补充说明 description）+ 实心危险确认键；
 *   danger=false 时为普通确认（accent 徽章 + 主色确认键）。
 */
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { pushEscHandler } from '../../utils/escStack'
import { useLocaleStore } from '../../stores/locale'
import Icon from './Icon.vue'

const props = withDefaults(
  defineProps<{
    title?: string
    /** 补充说明（如「删除后不可恢复」），与标题分层展示；不传则只显示标题。 */
    description?: string
    confirmText?: string
    cancelText?: string
    danger?: boolean
    disabled?: boolean
  }>(),
  { title: '', description: '', confirmText: '', cancelText: '', danger: true, disabled: false },
)

const locale = useLocaleStore()
const t = locale.t

/** 未传文案时按当前语言兜底。 */
const effectiveTitle = computed(() => props.title || t('confirm.title'))
const effectiveConfirm = computed(() => props.confirmText || t('confirm.ok'))
const effectiveCancel = computed(() => props.cancelText || t('common.cancel'))

const emit = defineEmits<{
  confirm: []
  cancel: []
}>()

const open = ref(false)
const triggerEl = ref<HTMLElement | null>(null)
const popEl = ref<HTMLElement | null>(null)
const pos = ref<{ left: number; top: number; up: boolean }>({ left: 0, top: 0, up: false })

const style = computed(() => ({
  left: `${pos.value.left}px`,
  top: `${pos.value.top}px`,
}))

function measure(): void {
  const el = triggerEl.value
  if (!el) return
  const rect = el.getBoundingClientRect()
  // 气泡高度随 description 增减，翻转判定按最大形态估算
  const height = props.description ? 130 : 104
  const up = window.innerHeight - rect.bottom - 8 < height && rect.top > height
  pos.value = {
    left: Math.min(rect.left, window.innerWidth - 296),
    top: up ? rect.top - height - 8 : rect.bottom + 8,
    up,
  }
}

function toggle(): void {
  if (props.disabled) return
  if (open.value) close()
  else {
    measure()
    open.value = true
  }
}

/**
 * 捕获阶段监听触发区的点击。
 *
 * 改用捕获而非模板里的 `@click.stop`：插槽内容可能自带 `@click.stop`（行内按钮
 * 阻止触发行点击），那会在目标阶段截断冒泡，外层永远收不到事件。捕获阶段在
 * 冒泡之前执行，无论插槽怎么 stop 都能收到。
 */
function onTriggerClick(event: Event): void {
  const target = event.target as Node | null
  if (!target || !triggerEl.value?.contains(target)) return
  toggle()
}

onMounted(() => {
  // 捕获阶段：第三个参数必须为 true
  triggerEl.value?.addEventListener('click', onTriggerClick, true)
})

function close(): void {
  open.value = false
}

function onConfirm(): void {
  close()
  emit('confirm')
}

function onCancel(): void {
  close()
  emit('cancel')
}

function onDocMouseDown(event: MouseEvent): void {
  const target = event.target as Node
  if (triggerEl.value?.contains(target) || popEl.value?.contains(target)) return
  close()
}

/** escStack 卸载函数（打开期间持有）。 */
let offEsc: (() => void) | null = null

watch(open, (isOpen) => {
  if (isOpen) {
    document.addEventListener('mousedown', onDocMouseDown, true)
    // ESC 层级仲裁：后开先关（Modal 里的确认气泡按 ESC 只关气泡）。
    offEsc = pushEscHandler(() => {
      close()
      return true
    })
  } else {
    document.removeEventListener('mousedown', onDocMouseDown, true)
    offEsc?.()
    offEsc = null
  }
})

onBeforeUnmount(() => {
  document.removeEventListener('mousedown', onDocMouseDown, true)
  offEsc?.()
  offEsc = null
  triggerEl.value?.removeEventListener('click', onTriggerClick, true)
})
</script>

<template>
  <span ref="triggerEl" class="pc-trigger">
    <slot />
    <Teleport to="body">
      <div
        v-if="open"
        ref="popEl"
        class="pc-pop"
        :class="{ up: pos.up }"
        :style="style"
        role="alertdialog"
        @click.stop
      >
        <div class="pc-head">
          <span class="pc-icon" :class="{ accent: !danger }">
            <Icon name="alert-triangle" :size="15" :stroke-width="1.75" />
          </span>
          <p class="pc-title">{{ effectiveTitle }}</p>
        </div>
        <p v-if="description" class="pc-desc">{{ description }}</p>
        <div class="pc-actions">
          <button class="miyo-btn miyo-btn-sm" type="button" @click="onCancel">{{ effectiveCancel }}</button>
          <button
            class="miyo-btn miyo-btn-sm"
            :class="danger ? 'miyo-btn-danger-solid' : 'miyo-btn-primary'"
            type="button"
            autofocus
            @click="onConfirm"
          >
            {{ effectiveConfirm }}
          </button>
        </div>
      </div>
    </Teleport>
  </span>
</template>

<style scoped>
.pc-trigger {
  display: inline-flex;
}

.pc-pop {
  position: fixed;
  z-index: var(--z-popconfirm);
  width: 288px;
  background: var(--bg-elevated);
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-lg);
  padding: 12px 14px;
  animation: pc-in 120ms var(--ease);
  transform-origin: top center;
}
.pc-pop.up {
  transform-origin: bottom center;
}

.pc-head {
  display: flex;
  align-items: center;
  gap: 10px;
}

/* 危险图标徽章：danger-tint 圆角底 + 危险色图标 */
.pc-icon {
  width: 28px;
  height: 28px;
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: var(--radius);
  background: var(--danger-tint);
  color: var(--danger);
}

.pc-icon.accent {
  background: var(--accent-tint);
  color: var(--accent);
}

.pc-title {
  margin: 0;
  font-size: 13px;
  font-weight: 600;
  line-height: 1.5;
  color: var(--text-1);
  word-break: break-all;
}

/* 补充说明：与标题文字对齐（28px 徽章 + 10px 间距） */
.pc-desc {
  margin: 6px 0 0 38px;
  font-size: 12px;
  line-height: 1.5;
  color: var(--text-2);
}

.pc-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 12px;
}

@keyframes pc-in {
  from {
    opacity: 0;
    transform: translateY(-4px) scale(0.97);
  }
  to {
    opacity: 1;
    transform: translateY(0) scale(1);
  }
}
</style>
