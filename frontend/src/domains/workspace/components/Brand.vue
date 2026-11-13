<script setup lang="ts">
/**
 * Brand：左上角品牌区（logo + 标题），点击返回主页。
 * 标题支持省略号截断，深/浅双主题，hover/focus 五态。
 */
import { useRouter } from 'vue-router'
import logo from '@/assets/miyo-logo.svg'
import { useLocaleStore } from '@/stores/locale'

withDefaults(defineProps<{ title: string; subtitle?: string }>(), { subtitle: '' })

const router = useRouter()
const t = useLocaleStore().t
</script>

<template>
  <button type="button" class="brand" :title="t('app.backHome')" @click="router.push('/projects')">
    <span class="brand-logo" aria-hidden="true">
      <img :src="logo" alt="" width="18" height="18" />
    </span>
    <span class="brand-text">
      <span class="brand-title" v-tooltip-overflow>{{ title }}</span>
      <span v-if="subtitle" class="brand-subtitle" v-tooltip-overflow>{{ subtitle }}</span>
    </span>
  </button>
</template>

<style scoped>
.brand {
  display: inline-flex;
  align-items: center;
  gap: 9px;
  min-width: 0;
  flex: 1;
  padding: 4px 6px;
  border: none;
  background: none;
  border-radius: var(--radius);
  cursor: pointer;
  text-align: left;
  transition: background var(--dur) var(--ease);
}
.brand:hover {
  background: var(--bg-hover);
}
.brand:active {
  background: var(--bg-hover);
}
.brand:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 1px;
}

.brand-logo {
  flex-shrink: 0;
  width: 28px;
  height: 28px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: #fff;
  border-radius: var(--radius-md);
  background: var(--accent);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.25);
}

.brand-text {
  display: flex;
  flex-direction: column;
  min-width: 0;
  gap: 0;
}

.brand-title {
  font-size: 13px;
  font-weight: 600;
  line-height: 1.3;
  color: var(--text-1);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.brand-subtitle {
  font-size: var(--fs-xxs);
  line-height: 1.3;
  color: var(--text-3);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
