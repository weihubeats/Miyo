<script setup lang="ts">
/**
 * JavaOnboarding：无项目 / 首次进入时的全页接入向导（替代空白三栏）。
 *
 * 三个入口（本地扫描 / 远程手动 / SDK 教程）+ 扫描候选列表 + 支持范围 +
 * 接入步骤 + FAQ。本地「扫描发现」是真实探测：SDK 注册文件 + 端口指纹
 * （401 not_authorized），候选由后端 java_discover_local 返回，不做任何
 * 假数据填充；无候选时明确引导手动接入。
 */
import { onMounted, ref } from 'vue'
import Icon from '@/components/ui/Icon.vue'
import IconButton from '@/components/ui/IconButton.vue'
import Spinner from '@/components/ui/Spinner.vue'
import { useJavaDebugStore } from '@/domains/java-debug/stores/javaDebug'
import { useLocaleStore } from '@/stores/locale'
import type { JavaDiscoveredRuntime } from '@/types/miyoApi.d'

const emit = defineEmits<{
  /** 远程接入：打开项目注册弹窗。 */
  'new-remote': []
  /** 打开 SDK 集成教程。 */
  'open-guide': []
  /** 采用某个本地候选（预填注册弹窗）。 */
  'use-candidate': [runtime: JavaDiscoveredRuntime]
}>()

const locale = useLocaleStore()
const t = locale.t
const store = useJavaDebugStore()

const scanning = ref(false)
const scanned = ref(false)

/** 扫描本机 Runtime（自动发现 = 后端真实探测，无候选即无候选）。 */
async function scan(): Promise<void> {
  scanning.value = true
  try {
    await store.discover()
    scanned.value = true
  } finally {
    scanning.value = false
  }
}

onMounted(() => {
  void scan()
})

const aliveCandidates = (): JavaDiscoveredRuntime[] => store.discovered.filter((c) => c.alive)
const staleCandidates = (): JavaDiscoveredRuntime[] => store.discovered.filter((c) => !c.alive)

const faqs = (): { q: string; a: string }[] => [
  { q: t('java.onboardingFaqQ1'), a: t('java.onboardingFaqA1') },
  { q: t('java.onboardingFaqQ2'), a: t('java.onboardingFaqA2') },
  { q: t('java.onboardingFaqQ3'), a: t('java.onboardingFaqA3') },
  { q: t('java.onboardingFaqQ4'), a: t('java.onboardingFaqA4') },
]

const steps = (): string[] => [
  t('java.onboardingStep1'),
  t('java.onboardingStep2'),
  t('java.onboardingStep3'),
  t('java.onboardingStep4'),
]
</script>

<template>
  <div class="job" data-testid="java-onboarding">
    <div class="job-scroll">
      <div class="job-inner">
        <!-- Hero -->
        <div class="job-hero">
          <div class="job-hero-icon"><Icon name="package" :size="28" /></div>
          <h1 class="job-title">{{ t('java.onboardingTitle') }}</h1>
          <p class="job-subtitle">{{ t('java.onboardingSubtitle') }}</p>
        </div>

        <!-- 三入口 -->
        <div class="job-entries">
          <button type="button" class="job-entry" data-testid="java-onboard-local" @click="scan()">
            <span class="job-entry-icon"><Icon name="search" :size="18" /></span>
            <span class="job-entry-name">{{ t('java.onboardingLocal') }}</span>
            <span class="job-entry-desc">{{ t('java.onboardingLocalDesc') }}</span>
          </button>
          <button type="button" class="job-entry" data-testid="java-onboard-remote" @click="emit('new-remote')">
            <span class="job-entry-icon"><Icon name="plug" :size="18" /></span>
            <span class="job-entry-name">{{ t('java.onboardingRemote') }}</span>
            <span class="job-entry-desc">{{ t('java.onboardingRemoteDesc') }}</span>
          </button>
          <button type="button" class="job-entry" data-testid="java-onboard-guide" @click="emit('open-guide')">
            <span class="job-entry-icon"><Icon name="code" :size="18" /></span>
            <span class="job-entry-name">{{ t('java.onboardingGuide') }}</span>
            <span class="job-entry-desc">{{ t('java.onboardingGuideDesc') }}</span>
          </button>
        </div>

        <!-- 本地扫描结果 -->
        <section class="job-scan">
          <div class="job-scan-head">
            <h2 class="job-section-title">
              {{
                scanning
                  ? t('java.onboardingScan')
                  : store.discovered.length > 0
                    ? t('java.onboardingCandidateTitle', { count: store.discovered.length })
                    : t('java.onboardingNoCandidate')
              }}
            </h2>
            <IconButton
              name="refresh"
              :title="t('java.onboardingScanAgain')"
              :disabled="scanning"
              data-testid="java-rescan"
              @click="scan()"
            />
          </div>

          <Spinner v-if="scanning" :size="16" class="job-scan-spin" />

          <template v-else-if="store.discovered.length > 0">
            <ul class="job-cand-list">
              <li
                v-for="c in aliveCandidates()"
                :key="c.endpoint"
                class="job-cand"
                data-testid="java-candidate"
              >
                <span class="job-cand-dot" />
                <span class="job-cand-endpoint">{{ c.endpoint }}</span>
                <span v-if="c.app_name" class="job-cand-meta">{{ c.app_name }}</span>
                <span v-if="c.java_version" class="job-cand-meta">Java {{ c.java_version }}</span>
                <span v-if="c.pid" class="job-cand-meta">PID {{ c.pid }}</span>
                <span class="job-cand-src">{{ c.source === 'registry' ? `${t('java.chipSdk')} ${c.sdk_version}` : '' }}</span>
                <button
                  type="button"
                  class="miyo-btn miyo-btn-primary job-cand-use"
                  @click="emit('use-candidate', c)"
                >
                  {{ t('java.onboardingUse') }}
                </button>
              </li>
              <li v-for="c in staleCandidates()" :key="c.endpoint" class="job-cand job-cand-stale">
                <span class="job-cand-dot job-cand-dot-stale" />
                <span class="job-cand-endpoint">{{ c.endpoint }}</span>
                <span class="job-cand-badge">{{ t('java.onboardingStaleBadge') }}</span>
                <span class="job-cand-meta">{{ t('java.onboardingStaleHint') }}</span>
              </li>
            </ul>
          </template>
          <p v-else-if="scanned" class="job-no-cand" data-testid="java-no-candidate">
            {{ t('java.onboardingNoCandidateHint') }}
          </p>
        </section>

        <!-- 支持范围 + 接入步骤 -->
        <div class="job-cols">
          <section class="job-col">
            <h2 class="job-section-title">{{ t('java.onboardingSupportTitle') }}</h2>
            <ul class="job-list">
              <li>{{ t('java.onboardingSupportJava') }}</li>
              <li>{{ t('java.onboardingSupportSpring') }}</li>
              <li>{{ t('java.onboardingSupportPlain') }}</li>
            </ul>
          </section>
          <section class="job-col">
            <h2 class="job-section-title">{{ t('java.onboardingStepsTitle') }}</h2>
            <ol class="job-steps">
              <li v-for="(s, i) in steps()" :key="i" class="job-step">
                <span class="job-step-no">{{ i + 1 }}</span>
                <span>{{ s }}</span>
              </li>
            </ol>
          </section>
        </div>

        <!-- FAQ -->
        <section class="job-faq">
          <h2 class="job-section-title">{{ t('java.onboardingFaqTitle') }}</h2>
          <details v-for="(f, i) in faqs()" :key="i" class="job-faq-item" :open="i === 0">
            <summary class="job-faq-q">{{ f.q }}</summary>
            <p class="job-faq-a">{{ f.a }}</p>
          </details>
        </section>
      </div>
    </div>
  </div>
</template>

<style scoped>
.job {
  height: 100%;
  min-height: 0;
  display: flex;
  flex-direction: column;
}
.job-scroll {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 32px 20px;
}
.job-inner {
  max-width: 860px;
  margin: 0 auto;
  display: flex;
  flex-direction: column;
  gap: 22px;
}
.job-hero {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  text-align: center;
  padding-top: 12px;
}
.job-hero-icon {
  width: 52px;
  height: 52px;
  border-radius: var(--radius-lg);
  display: grid;
  place-items: center;
  background: var(--bg-hover);
  color: var(--primary);
  margin-bottom: 4px;
}
.job-title {
  margin: 0;
  font-size: 19px;
  font-weight: 650;
  color: var(--text);
}
.job-subtitle {
  margin: 0;
  font-size: 12.5px;
  color: var(--text-secondary);
  max-width: 560px;
}
.job-entries {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 12px;
}
@media (max-width: 760px) {
  .job-entries {
    grid-template-columns: 1fr;
  }
}
.job-entry {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 6px;
  padding: 14px 16px;
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: var(--bg-elevated);
  color: var(--text);
  text-align: left;
  cursor: pointer;
}
.job-entry:hover {
  border-color: var(--primary);
  background: var(--bg-hover);
}
.job-entry-icon {
  color: var(--primary);
  display: inline-flex;
}
.job-entry-name {
  font-size: 13px;
  font-weight: 600;
}
.job-entry-desc {
  font-size: 11.5px;
  color: var(--text-secondary);
  line-height: 1.5;
}
.job-scan {
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  padding: 12px 14px;
  background: var(--bg-elevated);
}
.job-scan-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}
.job-section-title {
  margin: 0;
  font-size: 11.5px;
  font-weight: 600;
  color: var(--text-tertiary, var(--text-secondary));
  text-transform: uppercase;
  letter-spacing: 0.04em;
}
.job-scan-spin {
  margin: 10px auto;
}
.job-cand-list {
  list-style: none;
  margin: 10px 0 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.job-cand {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  font-size: 12px;
}
.job-cand-dot {
  width: 7px;
  height: 7px;
  border-radius: 999px;
  background: var(--ok, #22c55e);
  flex-shrink: 0;
}
.job-cand-dot-stale {
  background: var(--text-tertiary, var(--text-secondary));
}
.job-cand-stale {
  opacity: 0.75;
}
.job-cand-endpoint {
  font-family: var(--font-mono, monospace);
  font-weight: 600;
}
.job-cand-meta {
  color: var(--text-secondary);
  font-size: 11px;
}
.job-cand-badge {
  padding: 1px 7px;
  border-radius: 999px;
  font-size: 10px;
  background: var(--bg-hover);
  color: var(--text-secondary);
}
.job-cand-src {
  color: var(--text-tertiary, var(--text-secondary));
  font-size: 11px;
  margin-left: auto;
}
.job-cand-use {
  flex-shrink: 0;
}
.job-no-cand {
  margin: 10px 0 0;
  font-size: 12px;
  color: var(--text-secondary);
  line-height: 1.6;
}
.job-cols {
  display: grid;
  grid-template-columns: 1fr 1.2fr;
  gap: 12px;
}
@media (max-width: 760px) {
  .job-cols {
    grid-template-columns: 1fr;
  }
}
.job-col {
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  padding: 12px 14px;
  background: var(--bg-elevated);
}
.job-list {
  list-style: none;
  margin: 8px 0 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 7px;
  font-size: 12px;
  color: var(--text-secondary);
  line-height: 1.55;
}
.job-list li {
  padding-left: 14px;
  position: relative;
}
.job-list li::before {
  content: '';
  position: absolute;
  left: 0;
  top: 7px;
  width: 5px;
  height: 5px;
  border-radius: 999px;
  background: var(--primary);
}
.job-steps {
  list-style: none;
  margin: 8px 0 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 7px;
  font-size: 12px;
  color: var(--text-secondary);
  line-height: 1.55;
}
.job-step {
  display: flex;
  gap: 8px;
  align-items: baseline;
}
.job-step-no {
  width: 18px;
  height: 18px;
  border-radius: 999px;
  background: var(--bg-hover);
  color: var(--text-secondary);
  font-size: 11px;
  display: inline-grid;
  place-items: center;
  flex-shrink: 0;
}
.job-faq {
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  padding: 12px 14px;
  background: var(--bg-elevated);
}
.job-faq-item {
  border-top: 1px solid var(--border);
  padding: 8px 0;
}
.job-faq-item:first-of-type {
  border-top: none;
}
.job-faq-q {
  font-size: 12.5px;
  font-weight: 600;
  color: var(--text);
  cursor: pointer;
}
.job-faq-a {
  margin: 6px 0 0;
  font-size: 12px;
  color: var(--text-secondary);
  line-height: 1.65;
}
</style>
