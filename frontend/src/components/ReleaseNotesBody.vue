<script setup lang="ts">
/**
 * ReleaseNotesBody：版本更新说明渲染块（markdownLite 转义渲染）。
 * - 井底包裹框（--bg-code，深 #19191d / 浅 #f8fafc）+ 固定高度可滚动 + 紫点列表；
 * - 顶部更新 Tag 悬停预览（HeaderUpdateTag）与更新详情弹窗（ReleaseNotesModal）共用，
 *   两处排版只有 max-height 一处差异，避免同样的 Markdown 排版各写一份；
 * - 说明为空时展示兜底文案（release.noNotes）。
 */
import { computed } from 'vue'
import { useLocaleStore } from '../stores/locale'
import { renderMarkdownLite } from '../utils/markdownLite'

const props = withDefaults(
  defineProps<{
    /** 原始 Markdown 更新说明（空串 → 兜底文案）。 */
    notes: string
    /** 包裹框最大高度（超出滚动）。 */
    maxHeight?: string
  }>(),
  { maxHeight: '160px' },
)

const t = useLocaleStore().t

const notesHtml = computed(() => renderMarkdownLite(props.notes))
</script>

<template>
  <div class="rn-notes-box" :style="{ maxHeight }">
    <div v-if="notesHtml" class="rn-notes" v-html="notesHtml"></div>
    <p v-else class="rn-notes-empty">{{ t('release.noNotes') }}</p>
  </div>
</template>

<style scoped>
/* 日志包裹框：井底配色走主题变量（--bg-code 深 #19191d / 浅 #f8fafc），
   描边与文字走主题令牌，不再写死深色字面量类（浅色下原黑底框突兀） */
.rn-notes-box {
  overflow-y: auto;
  padding: 16px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--bg-code);
  color: var(--text-2);
  font-size: var(--fs-xs);
}

/* 空态（无更新说明） */
.rn-notes-empty {
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--text-3);
}

/* 段落/列表纵向行距（等价 space-y-2：非首子元素 8px 上边距，多行日志逐条呼吸） */
.rn-notes > :deep(* + *) {
  margin-top: 8px;
}

/* 无序列表：半透明紫点 marker（色随 --accent，字号/行高继承包裹框） */
.rn-notes :deep(ul > li::marker) {
  color: color-mix(in srgb, var(--accent) 55%, transparent);
}

/* 井底容器内的排版：文字令牌随主题（--text-*），深浅井底均可读 */
.rn-notes :deep(h3),
.rn-notes :deep(h4),
.rn-notes :deep(h5),
.rn-notes :deep(h6) {
  margin: 10px 0 4px;
  color: var(--text-1);
  font-size: 13px;
  font-weight: 600;
}
.rn-notes :deep(h3:first-child),
.rn-notes :deep(h4:first-child),
.rn-notes :deep(h5:first-child),
.rn-notes :deep(h6:first-child) {
  margin-top: 0;
}
.rn-notes :deep(p) {
  margin: 0 0 6px;
}
.rn-notes :deep(p:last-child) {
  margin-bottom: 0;
}
.rn-notes :deep(ul),
.rn-notes :deep(ol) {
  margin: 0 0 6px;
  padding-left: 18px;
}
.rn-notes :deep(li) {
  margin: 2px 0;
}
.rn-notes :deep(strong) {
  color: var(--text-1);
}
.rn-notes :deep(code) {
  padding: 1px 5px;
  border-radius: 4px;
  background: color-mix(in srgb, var(--text-1) 8%, transparent);
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
}
.rn-notes :deep(pre) {
  margin: 0 0 6px;
  padding: 8px;
  overflow-x: auto;
  border-radius: 6px;
  background: var(--bg-elevated);
}
.rn-notes :deep(pre code) {
  padding: 0;
  background: none;
}
.rn-notes :deep(a) {
  color: var(--accent);
  text-decoration: underline;
}
.rn-notes :deep(hr) {
  margin: 8px 0;
  border: none;
  border-top: 1px solid var(--border);
}
.rn-notes :deep(blockquote) {
  margin: 0 0 6px;
  padding-left: 8px;
  border-left: 2px solid var(--border-strong);
  color: var(--text-3);
}
</style>
