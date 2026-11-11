<script setup lang="ts">
/**
 * VarSuggest：{{变量}} 自动补全弹层。
 * Teleport 到 body + fixed 定位（仿 ui/Menu），避开祖先容器的 overflow 裁剪；
 * 根节点 @mousedown.prevent 阻止按下即失焦，点击候选项走 click 回写。
 */
import { nextTick, ref, watch } from 'vue'

const props = defineProps<{
  anchor: HTMLElement | null
  items: string[]
  activeIndex: number
}>()
const emit = defineEmits<{ pick: [index: number] }>()

const pos = ref({ left: 0, top: 0, width: 0 })
const listEl = ref<HTMLElement | null>(null)

/** 展示文本与插入内容一致：`{{name}}`（模板里内联反引号会被 `}}` 提前截断，故走函数）。 */
function itemName(item: string): string {
  return `{{${item}}}`
}

function place(): void {
  const el = props.anchor
  if (!el) return
  const rect = el.getBoundingClientRect()
  const width = Math.max(rect.width, 200)
  const maxLeft = window.innerWidth - width - 8
  const left = Math.min(Math.max(8, rect.left), Math.max(8, maxLeft))
  const est = Math.min(props.items.length, 8) * 26 + 12
  const below = rect.bottom + 4
  const top = below + est > window.innerHeight - 8 ? Math.max(8, rect.top - est - 4) : below
  pos.value = { left, top, width }
}

watch(() => props.items.length, () => place())
watch(() => props.anchor, () => place(), { immediate: true })

watch(
  () => props.activeIndex,
  () => {
    void nextTick(() => {
      const list = listEl.value
      const el = list?.children[props.activeIndex] as HTMLElement | undefined
      el?.scrollIntoView?.({ block: 'nearest' })
    })
  },
)
</script>

<template>
  <Teleport to="body">
    <div
      ref="listEl"
      class="vs-pop"
      :style="{ left: `${pos.left}px`, top: `${pos.top}px`, minWidth: `${pos.width}px` }"
      @mousedown.prevent
    >
      <button
        v-for="(item, i) in items"
        :key="item"
        type="button"
        class="vs-item"
        :class="{ 'vs-item-hl': i === activeIndex }"
        @click="emit('pick', i)"
      >
        <span class="vs-name">{{ itemName(item) }}</span>
      </button>
    </div>
  </Teleport>
</template>

<style scoped>
.vs-pop {
  position: fixed;
  z-index: var(--z-toast);
  display: flex;
  flex-direction: column;
  gap: 2px;
  max-height: 240px;
  overflow-y: auto;
  padding: 4px;
  background: var(--bg-elevated);
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-lg);
}

.vs-item {
  display: flex;
  align-items: center;
  width: 100%;
  padding: 5px 8px;
  border: none;
  background: none;
  border-radius: var(--radius-sm);
  font-family: var(--font-mono);
  font-size: 12px;
  color: var(--text-2);
  text-align: left;
  cursor: pointer;
  transition: background var(--dur) var(--ease);
}
.vs-item:hover {
  background: var(--bg-hover);
}
.vs-item-hl,
.vs-item-hl:hover {
  background: var(--accent-tint);
  color: var(--text-1);
}

.vs-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
