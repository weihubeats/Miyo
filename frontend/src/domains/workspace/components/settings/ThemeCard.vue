<script setup lang="ts">
/**
 * ThemeCard：主题预览卡（主题网格的 radiogroup 选项）。
 *
 * - 用真实色板渲染迷你窗口模型：页面背景铺底 + 卡片窗体 + 强调色条与文本行；
 * - 自定义主题卡可展示背景图缩略（imageSrc 存在时铺满预览区，色板退居兜底）；
 * - 选中态 = 强调色描边 + 名称行内对勾（克制，无大面积高亮）；
 * - 交互：点击选中；焦点管理（roving tabindex）由父级网格统一处理。
 */
import Icon from '@/components/ui/Icon.vue'

defineProps<{
  label: string
  swatch: { app: string; card: string; accent: string }
  selected: boolean
  /** 本卡是否持有焦点漫游位（父级 roving tabindex）。 */
  tabbable?: boolean
  /** 自定义主题背景图 dataURL（可选）。 */
  imageSrc?: string | null
}>()

const emit = defineEmits<{ select: [] }>()
</script>

<template>
  <button
    type="button"
    role="radio"
    :aria-checked="selected"
    :tabindex="tabbable ? 0 : -1"
    class="tc"
    :class="{ selected }"
    @click="emit('select')"
  >
    <span class="tc-preview" :style="{ background: swatch.app }">
      <span
        v-if="imageSrc"
        class="tc-image"
        :style="{ backgroundImage: `url('${imageSrc}')` }"
      />
      <span class="tc-window" :style="{ background: swatch.card }">
        <span class="tc-bar" :style="{ background: swatch.accent }" />
        <span class="tc-line" :style="{ background: swatch.accent, opacity: 0.35 }" />
        <span class="tc-line short" :style="{ background: swatch.accent, opacity: 0.2 }" />
      </span>
    </span>
    <span class="tc-meta">
      <span class="tc-name">{{ label }}</span>
      <Icon v-if="selected" name="check" :size="13" class="tc-check" aria-hidden="true" />
    </span>
  </button>
</template>

<style scoped>
.tc {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 6px;
  border: 1px solid var(--border);
  background: transparent;
  border-radius: var(--radius-md);
  cursor: pointer;
  text-align: left;
  font-family: inherit;
  transition:
    border-color var(--dur) var(--ease),
    background var(--dur) var(--ease);
}
.tc:hover {
  border-color: var(--border-strong);
  background: var(--bg-hover);
}
.tc:focus-visible {
  outline: 2px solid var(--focus-ring);
  outline-offset: -2px;
}
.tc.selected {
  border-color: var(--accent);
  background: var(--accent-tint);
}

/* 迷你窗口模型：背景铺满，卡片窗体浮起，强调条 + 两行「文本」
   注意：必须用 flex（而非 block）+ 显式 width——WebKit 对 display:flex 的
   <button> 内的 display:block 子元素不做伸展，宽度塌缩为 0（预览只剩
   边框塌成的一条细竖线，Chromium 正常），已实证；与 tc-meta 同用 flex 规避。 */
.tc-preview {
  position: relative;
  display: flex;
  flex-direction: column;
  width: 100%;
  height: 52px;
  border-radius: var(--radius-sm);
  overflow: hidden;
  border: 1px solid var(--border);
}
.tc-image {
  position: absolute;
  inset: 0;
  background-size: cover;
  background-position: center;
}
.tc-window {
  position: absolute;
  left: 10px;
  right: 10px;
  top: 12px;
  bottom: 8px;
  border-radius: var(--radius-sm);
  border: 1px solid var(--border);
  box-shadow: var(--shadow-sm);
  padding: 6px 7px;
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.tc-bar {
  width: 34%;
  height: 5px;
  border-radius: var(--radius-pill);
}
.tc-line {
  width: 78%;
  height: 3px;
  border-radius: var(--radius-pill);
}
.tc-line.short {
  width: 46%;
}

.tc-meta {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 6px;
  padding: 0 2px 1px;
}
.tc-name {
  font-size: var(--fs-xs);
  color: var(--text-1);
  line-height: 1.2;
}
.tc.selected .tc-name {
  font-weight: 500;
}
.tc-check {
  color: var(--accent);
  flex-shrink: 0;
}
</style>
