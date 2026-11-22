<script setup lang="ts">
/**
 * Brand：左上角品牌区（logo + 标题），点击返回主页。
 * 标题支持省略号截断，深/浅双主题，hover/focus 五态。
 */
import { computed } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import logo from '@/assets/miyo-logo.svg'
import { useLocaleStore } from '@/stores/locale'

withDefaults(defineProps<{ title: string; subtitle?: string }>(), { subtitle: '' })

const route = useRoute()
const router = useRouter()
const t = useLocaleStore().t

/**
 * 已在项目首页时品牌钮退化为纯展示：不再自指跳转 /projects，
 * 也不显示「回到项目首页」tooltip（自指导航入口收口）。
 */
const isHome = computed(() => route.path === '/projects')
</script>

<template>
  <button
    type="button"
    class="brand"
    :class="{ 'brand-home': isHome }"
    :title="isHome ? undefined : t('app.backHome')"
    :aria-disabled="isHome || undefined"
    @click="!isHome && router.push('/projects')"
  >
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
/* 首页自指收口：已是首页时品牌区为纯展示态 */
.brand-home {
  cursor: default;
}
.brand-home:hover {
  background: none;
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
  color: var(--text-on-accent);
  border-radius: var(--radius-md);
  background: var(--accent);
  box-shadow: none;
}

.brand-text {
  display: flex;
  flex-direction: column;
  min-width: 0;
  gap: 0;
}

.brand-title {
  font-size: var(--fs-sm);
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
