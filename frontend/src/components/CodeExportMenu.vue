<script setup lang="ts">
/**
 * CodeExportMenu：接口代码导出下拉菜单。
 *
 * 点击「导出代码」展开轻量 Popover（cURL / Go / Java / Python / Node.js），
 * 选择语言后后台生成代码并直接写入剪贴板：
 * - 成功：触发按钮短暂显示「✓ 已复制」（2 秒 + 弹跳动画）；
 * - 剪贴板不可用（非安全上下文 / 权限被拒）：降级 execCommand，仍失败则
 *   回退打开代码预览弹窗，保证代码不丢失。
 *
 * 入参为 codegen 原始输入（method/url/headers/body/auth），不耦合 Endpoint——
 * 调试页与测试用例抽屉（贴底工具条用 placement="top" 向上弹）均可复用。
 */
import { onBeforeUnmount, onMounted, ref } from 'vue'
import { useMiyoApi } from '../composables/useMiyoApi'
import { useToast } from '../composables/useToast'
import { useLocaleStore } from '../stores/locale'
import { copyText } from '../utils/clipboard'
import CodeExportDialog from './CodeExportDialog.vue'
import Icon from './ui/Icon.vue'
import type { AuthSpec, BodySpec, CodeLang, HttpMethod, KeyValue } from '../types/miyoApi'
import { errorText } from '../utils/errorText'

const props = defineProps<{
  method: HttpMethod
  url: string
  headers: KeyValue[]
  body: BodySpec
  auth: AuthSpec
  /** 置灰导出（如用例 Path 为空）。 */
  disabled?: boolean
  /** 菜单展开方向：默认向下；贴底工具条场景传 top 向上弹。 */
  placement?: 'bottom' | 'top'
  /** 水平对齐：默认右对齐（调试页顶栏靠右）；贴左侧触发器传 left 向右展开，与
   * CustomSelect 下拉一致，避免弹出层越出抽屉面板盖住主内容区。 */
  align?: 'left' | 'right'
}>()

/** 菜单选项（value 对应后端 codegen_render 的 lang）。 */
const CODE_EXPORT_OPTIONS: Array<{ value: CodeLang; label: string }> = [
  { value: 'curl', label: 'cURL' },
  { value: 'go', label: 'Go (net/http)' },
  { value: 'java', label: 'Java (OkHttp)' },
  { value: 'python', label: 'Python (requests)' },
  { value: 'js', label: 'Node.js (axios)' },
]

/** 卸载后丢弃异步结果，防止写状态 / 弹 Toast。 */
let disposed = false
let copiedTimer: ReturnType<typeof setTimeout> | null = null

onBeforeUnmount(() => {
  disposed = true
  window.removeEventListener('mousedown', onDocMousedown)
  window.removeEventListener('keydown', onDocKeydown)
  if (copiedTimer) clearTimeout(copiedTimer)
})

const api = useMiyoApi()
const toast = useToast()
const locale = useLocaleStore()
const t = locale.t

const wrapEl = ref<HTMLElement | null>(null)
const openMenu = ref(false)
/** 正在生成的语言（期间禁用全部选项，当前项显示旋转指示）。 */
const busyLang = ref<CodeLang | null>(null)
const copied = ref(false)
/** 剪贴板彻底不可用时的预览弹窗兜底。 */
const showFallback = ref(false)

/** 触发按钮闪烁 ✓ 已复制状态，2 秒后复位。 */
function flashCopied(): void {
  copied.value = true
  if (copiedTimer) clearTimeout(copiedTimer)
  copiedTimer = setTimeout(() => {
    copied.value = false
  }, 2000)
}

function toggleMenu(): void {
  openMenu.value = !openMenu.value
}

function onDocMousedown(event: MouseEvent): void {
  if (!openMenu.value) return
  const el = wrapEl.value
  if (el && event.target instanceof Node && !el.contains(event.target)) {
    openMenu.value = false
  }
}

function onDocKeydown(event: KeyboardEvent): void {
  if (openMenu.value && (event.key === 'Escape' || event.key === 'Tab')) {
    openMenu.value = false
  }
}

onMounted(() => {
  window.addEventListener('mousedown', onDocMousedown)
  window.addEventListener('keydown', onDocKeydown)
})

/** 生成指定语言代码并写入剪贴板；失败逐级降级。 */
async function pickLang(lang: CodeLang): Promise<void> {
  if (props.disabled || busyLang.value) return
  openMenu.value = false
  busyLang.value = lang
  try {
    const code = await api.codegenRender({
      lang,
      method: props.method,
      url: props.url,
      headers: props.headers,
      body: props.body,
      auth: props.auth,
    })
    if (disposed) return

    const ok = await copyText(code)
    if (disposed) return

    if (ok) {
      flashCopied()
    } else {
      toast.error(t('codegen.copyFailPreview'))
      showFallback.value = true
    }
  } catch (err) {
    if (disposed) return
    toast.error(t('codegen.genFail'), {
      message: errorText(err),
    })
  } finally {
    if (!disposed) busyLang.value = null
  }
}
</script>

<template>
  <div ref="wrapEl" class="code-export">
    <button
      type="button"
      class="miyo-btn ce-trigger"
      :class="{ 'ce-copied': copied }"
      :disabled="disabled"
      @click="toggleMenu"
    >
      <Icon v-if="copied" name="check" :size="14" />
      <Icon v-else name="code" :size="13" />
      {{ copied ? t('codegen.copied') : t('codegen.export') }}
      <Icon name="chevron-down" :size="11" class="ce-caret" :class="{ 'ce-caret-open': openMenu }" />
    </button>

    <div
      v-if="openMenu"
      class="ce-menu"
      :class="{ 'ce-menu-top': placement === 'top', 'ce-menu-left': align === 'left' }"
      role="menu"
    >
      <button
        v-for="opt in CODE_EXPORT_OPTIONS"
        :key="opt.value"
        type="button"
        role="menuitem"
        class="ce-item"
        :disabled="busyLang !== null"
        @click="pickLang(opt.value)"
      >
        <span>{{ opt.label }}</span>
        <Icon v-if="busyLang === opt.value" name="refresh" :size="12" class="ce-spin" />
      </button>
    </div>

    <CodeExportDialog
      v-if="showFallback"
      :method="method"
      :url="url"
      :headers="headers"
      :body="body"
      :auth="auth"
      @close="showFallback = false"
    />
  </div>
</template>

<style scoped>
.code-export {
  position: relative;
  display: inline-flex;
}

.ce-trigger {
  transition: color var(--dur) var(--ease), border-color var(--dur) var(--ease);
}
.ce-trigger.ce-copied {
  color: var(--success);
  border-color: var(--success-tint);
  animation: ce-check 220ms var(--ease);
}
.ce-trigger:disabled {
  opacity: 0.55;
  cursor: default;
}

.ce-caret {
  opacity: 0.6;
  transition:
    transform var(--dur) var(--ease),
    opacity var(--dur) var(--ease);
}
.ce-caret-open {
  transform: rotate(180deg);
  opacity: 1;
}

/* Popover 菜单 */
.ce-menu {
  position: absolute;
  top: calc(100% + 6px);
  right: 0;
  z-index: 60;
  min-width: 200px;
  padding: 4px;
  border: 1px solid var(--border);
  border-radius: 10px;
  background: var(--bg-panel);
  box-shadow: 0 8px 24px rgb(0 0 0 / 0.14);
  animation: ce-pop 130ms var(--ease);
}

/* 贴底工具条场景：从按钮上方展开（抽屉 Footer 等） */
.ce-menu.ce-menu-top {
  top: auto;
  bottom: calc(100% + 6px);
}

/* 贴左侧触发器场景：左对齐向右展开（留在所属面板内，与 CustomSelect 下拉一致） */
.ce-menu.ce-menu-left {
  left: 0;
  right: auto;
}

.ce-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  width: 100%;
  padding: 7px 10px;
  border: none;
  border-radius: var(--radius);
  background: transparent;
  color: var(--text-1);
  font-size: 12.5px;
  text-align: left;
  cursor: pointer;
  transition: background var(--dur) var(--ease);
}
.ce-item:hover:not(:disabled) {
  background: var(--bg-hover);
}
.ce-item:disabled {
  opacity: 0.55;
  cursor: default;
}

.ce-spin {
  flex-shrink: 0;
  animation: ce-spin 0.8s linear infinite;
}

@keyframes ce-pop {
  from {
    opacity: 0;
    transform: translateY(-4px) scale(0.98);
  }
}
@keyframes ce-spin {
  to {
    transform: rotate(360deg);
  }
}
@keyframes ce-check {
  0% {
    transform: scale(0.8);
  }
  60% {
    transform: scale(1.12);
  }
  100% {
    transform: scale(1);
  }
}
</style>