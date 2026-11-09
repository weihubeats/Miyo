<script setup lang="ts">
/**
 * TestCaseRunCompare：两次用例执行结果的并排对比。
 * - 摘要行：状态 / 状态码 / 耗时 / 断言通过数；
 * - 断言逐条 diff：按顺序配对（数量不同以 — 占位），预期 / 实际 / 结果。
 * 数据来自执行历史的 results_json 解析（EndpointResult）。
 */
import { computed } from 'vue'
import type { EndpointResult } from '../../types/miyoApi'
import { formatDuration } from '../../utils/testCases'
import { useLocaleStore } from '../../stores/locale'

const props = defineProps<{
  a: EndpointResult
  b: EndpointResult
  labelA: string
  labelB: string
}>()

const t = useLocaleStore().t

/** 断言按顺序配对（名称优先，缺位 —）。 */
const rows = computed(() => {
  const max = Math.max(props.a.outcomes.length, props.b.outcomes.length)
  const out: { a?: (typeof props.a.outcomes)[number]; b?: (typeof props.b.outcomes)[number]; changed: boolean }[] = []
  for (let i = 0; i < max; i += 1) {
    const oa = props.a.outcomes[i]
    const ob = props.b.outcomes[i]
    out.push({
      a: oa,
      b: ob,
      changed: !!oa && !!ob && oa.passed !== ob.passed,
    })
  }
  return out
})

function summary(r: EndpointResult): string {
  const parts = [
    r.status != null ? `HTTP ${r.status}` : '—',
    formatDuration(r.duration_ms ?? 0),
    `${r.outcomes.filter((o) => o.passed).length}/${r.outcomes.length}`,
  ]
  return parts.join(' · ')
}
</script>

<template>
  <div class="cmp">
    <div class="cmp-cols">
      <div class="cmp-col">
        <div class="cmp-label">{{ labelA }}</div>
        <div class="cmp-summary">{{ summary(a) }}</div>
      </div>
      <div class="cmp-col">
        <div class="cmp-label">{{ labelB }}</div>
        <div class="cmp-summary">{{ summary(b) }}</div>
      </div>
    </div>
    <div v-if="rows.length" class="cmp-rows">
      <div class="cmp-row cmp-head">
        <span class="cmp-cell-name">{{ t('caselog.colAssert') }}</span>
        <span class="cmp-cell">{{ labelA }}</span>
        <span class="cmp-cell">{{ labelB }}</span>
      </div>
      <div v-for="(r, i) in rows" :key="i" class="cmp-row" :class="{ changed: r.changed }">
        <span class="cmp-cell-name" :title="r.a?.description ?? r.b?.description">
          {{ r.a?.description ?? r.b?.description ?? '—' }}
        </span>
        <span class="cmp-cell">
          <template v-if="r.a">
            <span class="cmp-pass" :class="{ fail: !r.a.passed }">{{ r.a.passed ? '✓' : '✕' }}</span>
            <span class="cmp-val">{{ r.a.actual ?? r.a.expected ?? '' }}</span>
          </template>
          <template v-else>—</template>
        </span>
        <span class="cmp-cell">
          <template v-if="r.b">
            <span class="cmp-pass" :class="{ fail: !r.b.passed }">{{ r.b.passed ? '✓' : '✕' }}</span>
            <span class="cmp-val">{{ r.b.actual ?? r.b.expected ?? '' }}</span>
          </template>
          <template v-else>—</template>
        </span>
      </div>
    </div>
    <p v-else class="cmp-empty">{{ t('caselog.noAsserts') }}</p>
  </div>
</template>

<style scoped>
.cmp {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.cmp-cols {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 8px;
}

.cmp-col {
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  padding: 8px 10px;
  background: var(--bg-2);
}

.cmp-label {
  font-size: var(--fs-xxs);
  color: var(--text-3);
  margin-bottom: 2px;
}

.cmp-summary {
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  color: var(--text-1);
}

.cmp-rows {
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  overflow: hidden;
}

.cmp-row {
  display: grid;
  grid-template-columns: minmax(120px, 1.4fr) 1fr 1fr;
  gap: 8px;
  padding: 5px 10px;
  font-size: var(--fs-xs);
  border-top: 1px solid var(--border);
  align-items: center;
}
.cmp-row:first-child {
  border-top: none;
}
.cmp-row.changed {
  background: color-mix(in srgb, var(--warning) 8%, transparent);
}

.cmp-head {
  background: var(--bg-2);
  color: var(--text-3);
  font-size: var(--fs-xxs);
}

.cmp-cell-name {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-2);
}

.cmp-cell {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  font-family: var(--font-mono);
}

.cmp-pass {
  flex-shrink: 0;
  color: var(--success);
}
.cmp-pass.fail {
  color: var(--danger);
}

.cmp-val {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-2);
}

.cmp-empty {
  font-size: var(--fs-xs);
  color: var(--text-3);
}
</style>
