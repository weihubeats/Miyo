<script setup lang="ts">
/**
 * StyleCard：视觉风格预览卡（风格网格的 radiogroup 选项）。
 *
 * 与 ThemeCard 的色板矩形不同：预览区用**真实组件语义 token** 渲染一个迷你
 * 界面（顶栏 / 卡片 / 输入框 / 主次按钮）——token 从 style.css 的
 * [data-style] 块按当前颜色主题解析，三套风格与 8 套色板、自定义主题的
 * 任意组合都所见即所得。
 *
 * 预览子树经 :data-style="styleId" 重新作用域化 token（CSS 变量继承自最近
 * 祖先），与页面其余部分互不影响；选中态/焦点管理同 ThemeCard。
 */
import { useId } from 'vue'

defineProps<{
  label: string
  description: string
  /** 预览子树作用的风格档（studio/minimal/soft）。 */
  styleId: 'studio' | 'minimal' | 'soft'
  selected: boolean
  /** 本卡是否持有焦点漫游位（父级 roving tabindex）。 */
  tabbable?: boolean
}>()

const emit = defineEmits<{ select: [] }>()

const descId = useId()
</script>

<template>
  <button
    type="button"
    role="radio"
    :aria-checked="selected"
    :aria-describedby="descId"
    :tabindex="tabbable ? 0 : -1"
    class="sc"
    :class="{ selected }"
    @click="emit('select')"
  >
    <span class="sc-preview" :data-style="styleId" aria-hidden="true">
      <span class="sc-window">
        <span class="sc-toolbar">
          <span class="sc-dot" /><span class="sc-dot" /><span class="sc-dot" />
          <span class="sc-bar" />
        </span>
        <span class="sc-body">
          <span class="sc-card">
            <span class="sc-line" />
            <span class="sc-input" />
            <span class="sc-actions">
              <span class="sc-btn primary" />
              <span class="sc-btn" />
            </span>
          </span>
        </span>
      </span>
    </span>
    <span class="sc-meta">
      <span class="sc-name">{{ label }}</span>
      <span :id="descId" class="sc-desc">{{ description }}</span>
    </span>
  </button>
</template>

<style scoped>
.sc {
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
.sc:hover {
  border-color: var(--border-strong);
  background: var(--bg-hover);
}
.sc:focus-visible {
  outline: 2px solid var(--focus-ring);
  outline-offset: -2px;
}
.sc.selected {
  border-color: var(--accent);
  background: var(--accent-tint);
}

/* ---- 迷你界面：全部消费组件语义 token（flex 子元素显式 width，防 WKWebView 塌缩） ---- */
.sc-preview {
  display: flex;
  flex-direction: column;
  width: 100%;
  height: 74px;
  border-radius: var(--radius-sm);
  overflow: hidden;
  border: 1px solid var(--border);
  background: var(--bg-app);
}
.sc-window {
  flex: 1;
  display: flex;
  flex-direction: column;
  width: 100%;
  margin: 5px;
  background: var(--bg-panel);
  border: 1px solid var(--toolbar-border);
  border-radius: var(--radius-sm);
  overflow: hidden;
}
.sc-toolbar {
  display: flex;
  align-items: center;
  gap: 3px;
  width: 100%;
  padding: 3px 5px;
  border-bottom: 1px solid var(--toolbar-border);
  background: var(--bg-panel);
}
.sc-dot {
  width: 4px;
  height: 4px;
  flex: none;
  border-radius: var(--radius-pill);
  background: var(--text-3);
  opacity: 0.55;
}
.sc-bar {
  flex: 1;
  width: auto;
  height: 4px;
  margin-left: 3px;
  border-radius: var(--radius-pill);
  background: var(--accent);
  opacity: 0.4;
}
.sc-body {
  flex: 1;
  display: flex;
  width: 100%;
  padding: 5px;
}
.sc-card {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 4px;
  width: 100%;
  padding: 6px;
  background: var(--card-bg);
  border: 1px solid var(--card-border);
  border-radius: var(--card-radius);
  box-shadow: var(--card-shadow);
}
.sc-line {
  width: 62%;
  height: 4px;
  border-radius: var(--radius-pill);
  background: var(--text-2);
  opacity: 0.55;
}
.sc-input {
  width: 100%;
  height: 9px;
  border: 1px solid var(--input-border);
  border-radius: var(--radius-sm);
  background: var(--input-bg);
}
.sc-actions {
  display: flex;
  gap: 4px;
  width: 100%;
}
.sc-btn {
  width: 22px;
  height: 9px;
  border-radius: calc(var(--radius-sm) + 1px);
  border: 1px solid var(--btn-border);
  background: var(--btn-bg);
}
.sc-btn.primary {
  width: 30px;
  border-color: transparent;
  background: var(--grad-primary);
  box-shadow: 0 1px 4px var(--primary-glow);
}

/* ---- 文案区 ---- */
.sc-meta {
  display: flex;
  flex-direction: column;
  gap: 1px;
  padding: 0 2px 1px;
  min-width: 0;
}
.sc-name {
  font-size: var(--fs-xs);
  color: var(--text-1);
  line-height: 1.2;
}
.sc.selected .sc-name {
  font-weight: 500;
}
.sc-desc {
  font-size: var(--fs-2xxs);
  color: var(--text-3);
  line-height: 1.35;
}
</style>
