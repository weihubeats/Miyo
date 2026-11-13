<script setup lang="ts">
/**
 * DeleteConfirmModal：删除确认居中弹窗（接口 / 文件夹删除统一落地）。
 * - 柔和模糊遮罩（backdrop-blur-sm，遮罩色走 --mask 深/浅两套）+ 卡片容器
 *   （w-400 rounded-2xl p-6，配色随主题令牌，浅色白底——原「双主题恒深色
 *   变量覆写」浅色下产出黑底弹窗，已移除，同 ReleaseNotesModal 先例）；
 * - 布局：警告徽章 + 基础标题（「删除接口」）+ 句式提示（含名称与不可撤销）+
 *   method/path 语义化代码卡片（方法色走 methodTone 单一真源）；
 * - 取消（Ghost）+ 确认删除（红实心 + 柔红晕）；Esc / 遮罩 / ✕ 可关。
 */
import { useLocaleStore } from '@/stores/locale'
import { methodTone } from '@/utils/methodTone'
import Icon from '@/components/ui/Icon.vue'
import Modal from '@/components/ui/Modal.vue'

defineProps<{
  open: boolean
  /** 基础标题（如「删除接口」/「删除文件夹」）。 */
  title: string
  /** 句式提示（含被删对象名称与不可撤销说明），由调用方经 i18n 组装。 */
  message: string
  /** HTTP 方法（接口删除时传入，渲染语义化徽章；空则不渲染代码卡片）。 */
  method?: string
  /** 请求路径（与 method 同现时渲染代码卡片）。 */
  path?: string
  /** 确认按钮文案（默认「确认删除」）。 */
  confirmLabel?: string
}>()
const emit = defineEmits<{ 'update:open': [open: boolean]; confirm: [] }>()

const t = useLocaleStore().t
</script>

<template>
  <Modal
    :open="open"
    width="400px"
    dialog-class="dc-modal"
    mask-class="backdrop-blur-sm"
    @update:open="emit('update:open', $event)"
  >
    <!-- 右上快捷关闭（容器 position: relative 定位，见文末样式） -->
    <button
      class="dc-close"
      type="button"
      :aria-label="t('common.close')"
      @click="emit('update:open', false)"
    >
      <Icon name="x" :size="14" />
    </button>
    <div class="dc-head">
      <span class="dc-badge"><Icon name="alert-triangle" :size="16" /></span>
      <div class="dc-texts">
        <p class="dc-title">{{ title }}</p>
        <p class="dc-message">{{ message }}</p>
        <!-- method/path 代码卡片：方法徽章走 methodTone（@theme 方法色单源），路径等宽高亮 -->
        <div v-if="method && path" class="dc-code">
          <span class="dc-method" :class="methodTone(method)">{{ method }}</span>
          <span class="dc-path">{{ path }}</span>
        </div>
      </div>
    </div>

    <template #footer>
      <div class="dc-actions">
        <button class="dc-cancel" type="button" @click="emit('update:open', false)">
          {{ t('common.cancel') }}
        </button>
        <button class="dc-confirm" type="button" @click="emit('confirm')">
          {{ confirmLabel || t('confirm.delete') }}
        </button>
      </div>
    </template>
  </Modal>
</template>

<style scoped>
.dc-head {
  display: flex;
  align-items: flex-start;
  gap: 16px;
}

/* 警告徽章：圆角红底 red-500/15 + 危险图标色（--danger-fg 随主题） */
.dc-badge {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  padding: 8px;
  border-radius: 12px;
  background: rgba(239, 68, 68, 0.15);
  color: var(--danger-fg, #f87171);
}

.dc-texts {
  min-width: 0;
  /* 让位右上角关闭图标 */
  padding-right: 16px;
}

.dc-title {
  margin: 0;
  font-size: 16px;
  font-weight: 700;
  color: var(--text-1);
  word-break: break-word;
}

.dc-message {
  margin: 4px 0 0;
  font-size: 12px;
  line-height: 1.55;
  color: var(--text-2);
  word-break: break-word;
}

/* method/path 代码卡片：井底容器（--bg-code 随主题） + 方法徽章 + 等宽路径 */
.dc-code {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 12px;
  padding: 10px;
  border: 1px solid var(--border);
  border-radius: 12px;
  background: var(--bg-code);
}

.dc-method {
  flex-shrink: 0;
  padding: 2px 8px;
  border-width: 1px;
  border-style: solid; /* 描边色由 methodTone 的 border-method-*／20 类提供，此处不设色避免压过工具类 */
  border-radius: 6px;
  font-family: var(--font-mono);
  font-size: 12px;
  font-weight: 600;
}

.dc-path {
  min-width: 0;
  overflow: hidden;
  font-family: var(--font-mono);
  font-size: 12px;
  color: var(--text-2);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.dc-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  width: 100%;
}

/* 右上快捷关闭：随主题（--text-3 → hover --text-1） */
.dc-close {
  position: absolute;
  top: 16px;
  right: 16px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  padding: 2px;
  border: none;
  background: none;
  color: var(--text-3);
  cursor: pointer;
  transition: color var(--dur) var(--ease);
}
.dc-close:hover {
  color: var(--text-1);
}

.dc-cancel {
  padding: 6px 14px;
  border: none;
  border-radius: 8px;
  background: none;
  font-family: inherit;
  font-size: 12px;
  font-weight: 500;
  color: var(--text-2);
  cursor: pointer;
  transition: all var(--dur) var(--ease);
}
.dc-cancel:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}

/* 高危按钮：红底 + 红描边 + 柔化红晕（原深红双层阴影浅色下糊成脏块），减弱纯红刺眼感 */
.dc-confirm {
  padding: 6px 16px;
  border: 1px solid rgba(239, 68, 68, 0.3);
  border-radius: 8px;
  background: var(--danger);
  font-family: inherit;
  font-size: 12px;
  font-weight: 500;
  color: #fff;
  box-shadow: 0 8px 16px -8px rgba(220, 38, 38, 0.5);
  cursor: pointer;
  transition: all var(--dur) var(--ease);
}
.dc-confirm:hover {
  background: color-mix(in srgb, var(--danger) 88%, #fff);
}
</style>

<style>
/* 遮罩/容器经 Teleport 挂 body，覆写须非 scoped；body 前缀压过 Modal 的 scoped 规则。
   类名 dc-modal 仅本弹窗使用。配色一律不覆写：容器/文字随主题令牌，
   遮罩走 Modal 自带 var(--mask)（深/浅各一套）——原恒深底 token 覆写已移除。 */

/* 容器：仅覆写圆角（16px 结构性设计）+ 关闭图标定位锚点 + 背景毛玻璃 */
.dc-modal {
  --radius-lg: 16px;
  position: relative;
  backdrop-filter: blur(12px);
  -webkit-backdrop-filter: blur(12px);
}

/* p-6：内边距统一收到容器，内部两段归零 */
body .dc-modal.m-dialog {
  padding: 24px;
}
body .dc-modal .m-body {
  padding: 0;
}
body .dc-modal .m-foot {
  padding: 0;
  margin-top: 16px;
}
</style>
