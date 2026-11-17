<script setup lang="ts">
/**
 * ThemeAppearance：设置 → 通用设置 → 主题外观。
 *
 * 结构：模式分段（跟随系统 / 固定主题）→ 主题预览卡网格（6 内置 + 自定义入口，
 * radiogroup + 方向键漫游）→ 自定义主题编辑器（选中「自定义」时展开：基调 /
 * 五色 / 背景四形态 / 卡片透明度 / 圆角 / 字号）→ 导入 / 导出 / 恢复默认主题。
 *
 * 所有改动经 theme store 立即写 DOM（实时预览，免刷新）；背景图上传走
 * theme/backgroundImage.ts 压缩管线，错误经稳定 code 映射 i18n 文案。
 */
import { computed, ref } from 'vue'
import Icon from '@/components/ui/Icon.vue'
import SegmentedControl from '@/components/ui/SegmentedControl.vue'
import { useThemeStore } from '@/stores/theme'
import { useLocaleStore } from '@/stores/locale'
import { useToast } from '@/composables/useToast'
import { BUILTIN_THEMES } from '@/theme/presets'
import { processImageFile, type ImageProcessErrorCode } from '@/theme/backgroundImage'
import type { BackgroundType, CustomColors, ImageBackground, ThemeId } from '@/theme/types'
import ThemeCard from './ThemeCard.vue'

const theme = useThemeStore()
const locale = useLocaleStore()
const t = locale.t
const toast = useToast()

/* ---- 模式分段（数组随语言刷新，必须包 computed） ---- */
const modeOptions = computed(() => [
  { value: 'system', label: t('settings.themeSystem') },
  { value: 'manual', label: t('settings.themeManual') },
])

const onModeChange = (v: string): void => {
  theme.setMode(v === 'system' ? 'system' : 'manual')
}

/* ---- 主题名与网格项 ---- */
const THEME_NAME_KEYS: Record<Exclude<ThemeId, 'custom'>, string> = {
  light: 'settings.themeNameLight',
  dark: 'settings.themeNameDark',
  ocean: 'settings.themeNameOcean',
  forest: 'settings.themeNameForest',
  sand: 'settings.themeNameSand',
  lavender: 'settings.themeNameLavender',
}
const labelOf = (id: ThemeId): string =>
  id === 'custom' ? t('settings.themeNameCustom') : t(THEME_NAME_KEYS[id])

interface GridItem {
  id: ThemeId
  label: string
  swatch: { app: string; card: string; accent: string }
  imageSrc: string | null
}

const gridItems = computed<GridItem[]>(() => {
  const custom = theme.custom
  const baseSwatch =
    BUILTIN_THEMES.find((p) => p.id === (custom?.base === 'dark' ? 'dark' : 'light'))?.swatch ?? {
      app: '#f4f5f6',
      card: '#ffffff',
      accent: '#6d5ce6',
    }
  const customSwatch = custom
    ? {
        app: custom.colors.bg ?? baseSwatch.app,
        card: custom.colors.card ?? baseSwatch.card,
        accent: custom.colors.accent ?? baseSwatch.accent,
      }
    : baseSwatch
  const customImage =
    custom?.background.type === 'image' ? custom.background.image?.data ?? null : null
  return [
    ...BUILTIN_THEMES.map((p) => ({ id: p.id as ThemeId, label: labelOf(p.id), swatch: p.swatch, imageSrc: null })),
    { id: 'custom', label: t('settings.themeNameCustom'), swatch: customSwatch, imageSrc: customImage },
  ]
})

const isCustom = computed(() => theme.themeId === 'custom')
const custom = computed(() => theme.custom)

function selectTheme(id: ThemeId): void {
  theme.setThemeId(id)
}

/** 网格 roving tabindex：←/→/↑/↓/Home/End 移动焦点（选中由点击/Enter 完成）。 */
const GRID_KEYS = ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End']
const GRID_COLS = 4
function onGridKeydown(e: KeyboardEvent): void {
  if (!GRID_KEYS.includes(e.key)) return
  const root = e.currentTarget as HTMLElement
  const cards = [...root.querySelectorAll<HTMLButtonElement>('button[role="radio"]')]
  if (!cards.length) return
  e.preventDefault()
  let idx = cards.indexOf(document.activeElement as HTMLButtonElement)
  if (idx === -1) idx = 0
  if (e.key === 'ArrowRight') idx = (idx + 1) % cards.length
  else if (e.key === 'ArrowLeft') idx = (idx - 1 + cards.length) % cards.length
  else if (e.key === 'ArrowDown') idx = (idx + GRID_COLS) % cards.length
  else if (e.key === 'ArrowUp') idx = (idx - GRID_COLS + cards.length) % cards.length
  else if (e.key === 'Home') idx = 0
  else idx = cards.length - 1
  cards[idx]?.focus()
}

/** system 模式提示：当前系统深浅将使用哪套主题。 */
const systemHint = computed(() => {
  if (theme.mode !== 'system') return ''
  return theme.systemDark
    ? t('settings.themeSysDarkHint', { theme: labelOf(theme.config.lastDark) })
    : t('settings.themeSysLightHint', { theme: labelOf(theme.config.lastLight) })
})

/* ================= 自定义主题编辑器 ================= */

const baseOptions = computed(() => [
  { value: 'light', label: t('settings.themeBaseLight') },
  { value: 'dark', label: t('settings.themeBaseDark') },
])

const setBase = (v: string): void => {
  theme.updateCustom({ base: v === 'dark' ? 'dark' : 'light' })
}

/** 基调主题的默认色（未自定义时颜色行显示该值；清除即回到它）。 */
const baseDefaults = computed<Record<keyof CustomColors, string>>(() => {
  const dark = custom.value?.base === 'dark'
  const sw = BUILTIN_THEMES.find((p) => p.id === (dark ? 'dark' : 'light'))!.swatch
  return {
    bg: sw.app,
    card: sw.card,
    border: dark ? '#2a2a30' : '#e4e4e7',
    text: dark ? '#e9eaee' : '#202124',
    accent: sw.accent,
  }
})

const COLOR_ROWS: { key: keyof CustomColors; labelKey: string }[] = [
  { key: 'bg', labelKey: 'settings.themeColorPage' },
  { key: 'card', labelKey: 'settings.themeColorCard' },
  { key: 'border', labelKey: 'settings.themeColorBorder' },
  { key: 'text', labelKey: 'settings.themeColorText' },
  { key: 'accent', labelKey: 'settings.themeColorAccent' },
]

const colorValue = (key: keyof CustomColors): string =>
  custom.value?.colors[key] ?? baseDefaults.value[key]

const setColor = (key: keyof CustomColors, e: Event): void => {
  theme.updateCustom({ colors: { ...custom.value?.colors, [key]: targetValue(e) } })
}

const clearColor = (key: keyof CustomColors): void => {
  const next = { ...custom.value?.colors }
  delete next[key]
  theme.updateCustom({ colors: next })
}

/* ---- 背景 ---- */
const bgTypeOptions = computed(() => [
  { value: 'default', label: t('settings.themeBgDefault') },
  { value: 'color', label: t('settings.themeBgColor') },
  { value: 'gradient', label: t('settings.themeBgGradient') },
  { value: 'image', label: t('settings.themeBgImage') },
])

const bgType = computed<BackgroundType>(() => custom.value?.background.type ?? 'default')

const setBgType = (v: string): void => {
  const prev = custom.value?.background
  theme.updateCustom({
    background: { type: v as BackgroundType, color: prev?.color, gradient: prev?.gradient, image: prev?.image },
  })
}

const bgColor = computed(() => custom.value?.background.color ?? baseDefaults.value.bg)
const onBgColor = (e: Event): void => {
  theme.updateCustom({ background: { type: 'color', color: targetValue(e) } })
}

const GRADIENT_FALLBACK = { from: '#6d5ce6', to: '#1d1d21', angle: 135 }
const gradient = computed(() => custom.value?.background.gradient ?? GRADIENT_FALLBACK)
const setGradient = (patch: Partial<{ from: string; to: string; angle: number }>): void => {
  theme.updateCustom({
    background: { type: 'gradient', gradient: { ...GRADIENT_FALLBACK, ...gradient.value, ...patch } },
  })
}

const imageCfg = computed(() =>
  custom.value?.background.type === 'image' ? custom.value.background.image ?? null : null,
)

const IMAGE_ERROR_KEYS: Record<ImageProcessErrorCode, string> = {
  type: 'settings.themeImageTypeError',
  size: 'settings.themeImageSizeError',
  decode: 'settings.themeImageDecodeError',
  compress: 'settings.themeImageCompressError',
}

const imageInput = ref<HTMLInputElement | null>(null)
const uploading = ref(false)

async function onImagePick(e: Event): Promise<void> {
  const input = e.target as HTMLInputElement
  const file = input.files?.[0]
  input.value = '' // 允许重复选择同一文件
  if (!file) return
  uploading.value = true
  try {
    const res = await processImageFile(file)
    if (!res.ok) {
      toast.error(t(IMAGE_ERROR_KEYS[res.code]))
      return
    }
    const prev = imageCfg.value
    theme.updateCustom({
      background: {
        type: 'image',
        image: {
          data: res.dataUrl,
          fit: prev?.fit ?? 'cover',
          position: prev?.position ?? 'center',
          opacity: prev?.opacity ?? 1,
          blur: prev?.blur ?? 0,
        },
      },
    })
  } finally {
    uploading.value = false
  }
}

const setImage = (patch: Partial<ImageBackground>): void => {
  const prev = imageCfg.value
  if (!prev) return
  theme.updateCustom({ background: { type: 'image', image: { ...prev, ...patch } } })
}

const removeImage = (): void => {
  theme.updateCustom({ background: { type: 'default', image: null } })
}

const fitOptions = computed(() => [
  { value: 'cover', label: t('settings.themeFitCover') },
  { value: 'contain', label: t('settings.themeFitContain') },
])

const posOptions = computed(() => [
  { value: 'center', label: t('settings.themePosCenter') },
  { value: 'top', label: t('settings.themePosTop') },
  { value: 'bottom', label: t('settings.themePosBottom') },
  { value: 'left', label: t('settings.themePosLeft') },
  { value: 'right', label: t('settings.themePosRight') },
])

/* ---- 全局排版滑杆 ---- */
const cardOpacityPct = computed({
  get: () => Math.round((custom.value?.cardOpacity ?? 1) * 100),
  set: (v: number) => theme.updateCustom({ cardOpacity: v / 100 }),
})
const radiusPx = computed({
  get: () => custom.value?.radius ?? 6,
  set: (v: number) => theme.updateCustom({ radius: v }),
})
const fontScalePct = computed({
  get: () => Math.round((custom.value?.fontScale ?? 1) * 100),
  set: (v: number) => theme.updateCustom({ fontScale: v / 100 }),
})

/* ---- 导入 / 导出 / 恢复默认 ---- */
function exportTheme(): void {
  const blob = new Blob([theme.exportConfig()], { type: 'application/json' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = 'miyo-theme.json'
  a.click()
  URL.revokeObjectURL(url)
  toast.success(t('settings.themeExported'))
}

const importInput = ref<HTMLInputElement | null>(null)

function onImportPick(e: Event): void {
  const input = e.target as HTMLInputElement
  const file = input.files?.[0]
  input.value = ''
  if (!file) return
  void file.text().then((text) => {
    if (theme.importConfig(text)) toast.success(t('settings.themeImportOk'))
    else toast.error(t('settings.themeImportBad'))
  })
}

function resetTheme(): void {
  theme.resetTheme()
  toast.success(t('settings.themeResetAllDone'))
}

/* ---- 模板事件小工具（模板内不做 TS 断言） ---- */
function targetValue(e: Event): string {
  return (e.target as HTMLInputElement).value
}
function targetNumber(e: Event): number {
  return Number((e.target as HTMLInputElement).value)
}
</script>

<template>
  <div class="miyo-card p-5">
    <!-- 头部：标题 + 模式分段 -->
    <div class="flex items-center justify-between gap-4">
      <div class="min-w-0">
        <div class="text-sm font-medium text-ink1">{{ t('settings.theme') }}</div>
        <p class="mt-0.5 text-xs text-ink2">{{ t('settings.themeDesc') }}</p>
      </div>
      <SegmentedControl
        class="shrink-0"
        :model-value="theme.mode"
        :options="modeOptions"
        @change="onModeChange"
      />
    </div>
    <p v-if="systemHint" class="mt-2 text-xs text-ink3 num">{{ systemHint }}</p>

    <!-- 主题预览卡网格（radiogroup + 方向键漫游） -->
    <div
      class="mt-4 grid grid-cols-4 gap-2"
      role="radiogroup"
      :aria-label="t('settings.theme')"
      @keydown="onGridKeydown"
    >
      <ThemeCard
        v-for="item in gridItems"
        :key="item.id"
        :label="item.label"
        :swatch="item.swatch"
        :image-src="item.imageSrc"
        :selected="theme.themeId === item.id"
        :tabbable="theme.themeId === item.id || item.id === gridItems[0].id"
        @select="selectTheme(item.id)"
      />
    </div>

    <!-- 自定义主题编辑器（选中自定义时展开） -->
    <div v-if="isCustom && custom" class="ce mt-4 rounded-lg border border-line bg-code/60 p-4">
      <div class="mb-3 flex items-center justify-between gap-3">
        <div class="min-w-0">
          <div class="text-sm font-medium text-ink1">{{ t('settings.themeCustomize') }}</div>
          <p class="mt-0.5 text-xs text-ink2">{{ t('settings.themeCustomDesc') }}</p>
        </div>
      </div>

      <!-- 界面基调 -->
      <div class="ce-row">
        <span class="ce-label">{{ t('settings.themeCustomBase') }}</span>
        <SegmentedControl size="sm" :model-value="custom.base" :options="baseOptions" @change="setBase" />
      </div>

      <!-- 五个核心颜色 -->
      <div v-for="row in COLOR_ROWS" :key="row.key" class="ce-row">
        <span class="ce-label">{{ t(row.labelKey) }}</span>
        <div class="ce-color">
          <input
            type="color"
            class="ce-color-input"
            :value="colorValue(row.key)"
            @input="setColor(row.key, $event)"
          />
          <span class="ce-hex num">{{ colorValue(row.key) }}</span>
          <button
            v-if="custom.colors[row.key]"
            type="button"
            class="ce-mini-btn"
            :title="t('settings.themeColorReset')"
            :aria-label="t('settings.themeColorReset')"
            @click="clearColor(row.key)"
          >
            <Icon name="x" :size="11" aria-hidden="true" />
          </button>
        </div>
      </div>

      <!-- 背景形态 -->
      <div class="ce-row">
        <span class="ce-label">{{ t('settings.themeBackground') }}</span>
        <SegmentedControl size="sm" :model-value="bgType" :options="bgTypeOptions" @change="setBgType" />
      </div>

      <div v-if="bgType === 'color'" class="ce-row">
        <span class="ce-label">{{ t('settings.themeBgColor') }}</span>
        <div class="ce-color">
          <input type="color" class="ce-color-input" :value="bgColor" @input="onBgColor" />
          <span class="ce-hex num">{{ bgColor }}</span>
        </div>
      </div>

      <template v-else-if="bgType === 'gradient'">
        <div class="ce-row">
          <span class="ce-label">{{ t('settings.themeGradFrom') }}</span>
          <div class="ce-color">
            <input
              type="color"
              class="ce-color-input"
              :value="gradient.from"
              @input="setGradient({ from: targetValue($event) })"
            />
            <span class="ce-hex num">{{ gradient.from }}</span>
          </div>
        </div>
        <div class="ce-row">
          <span class="ce-label">{{ t('settings.themeGradTo') }}</span>
          <div class="ce-color">
            <input
              type="color"
              class="ce-color-input"
              :value="gradient.to"
              @input="setGradient({ to: targetValue($event) })"
            />
            <span class="ce-hex num">{{ gradient.to }}</span>
          </div>
        </div>
        <div class="ce-row">
          <span class="ce-label">{{ t('settings.themeGradAngle') }}</span>
          <div class="ce-slider-row">
            <input
              type="range"
              class="ce-range"
              min="0"
              max="360"
              :value="gradient.angle"
              @input="setGradient({ angle: targetNumber($event) })"
            />
            <span class="ce-val num">{{ gradient.angle }}°</span>
          </div>
        </div>
        <div
          class="ce-preview"
          :style="{ background: `linear-gradient(${gradient.angle}deg, ${gradient.from}, ${gradient.to})` }"
        />
      </template>

      <template v-else-if="bgType === 'image'">
        <div class="ce-row">
          <span class="ce-label">{{ t('settings.themeImageUpload') }}</span>
          <div class="flex items-center gap-2">
            <img v-if="imageCfg" :src="imageCfg.data" class="ce-thumb" alt="" />
            <button type="button" class="miyo-btn miyo-btn-sm" :disabled="uploading" @click="imageInput?.click()">
              <Icon name="upload" :size="12" aria-hidden="true" />
              {{ imageCfg ? t('settings.themeImageReplace') : t('settings.themeImageUpload') }}
            </button>
            <button v-if="imageCfg" type="button" class="miyo-btn miyo-btn-sm miyo-btn-ghost" @click="removeImage">
              {{ t('settings.themeImageRemove') }}
            </button>
          </div>
        </div>
        <template v-if="imageCfg">
          <div class="ce-row">
            <span class="ce-label">{{ t('settings.themeImageFit') }}</span>
            <SegmentedControl
              size="sm"
              :model-value="imageCfg.fit"
              :options="fitOptions"
              @change="(v) => setImage({ fit: v === 'contain' ? 'contain' : 'cover' })"
            />
          </div>
          <div class="ce-row">
            <span class="ce-label">{{ t('settings.themeImagePosition') }}</span>
            <SegmentedControl
              size="sm"
              :model-value="imageCfg.position"
              :options="posOptions"
              @change="(v) => setImage({ position: String(v) })"
            />
          </div>
          <div class="ce-row">
            <span class="ce-label">{{ t('settings.themeImageOpacity') }}</span>
            <div class="ce-slider-row">
              <input
                type="range"
                class="ce-range"
                min="0"
                max="100"
                :value="Math.round(imageCfg.opacity * 100)"
                @input="setImage({ opacity: targetNumber($event) / 100 })"
              />
              <span class="ce-val num">{{ Math.round(imageCfg.opacity * 100) }}%</span>
            </div>
          </div>
          <div class="ce-row">
            <span class="ce-label">{{ t('settings.themeImageBlur') }}</span>
            <div class="ce-slider-row">
              <input
                type="range"
                class="ce-range"
                min="0"
                max="24"
                :value="imageCfg.blur"
                @input="setImage({ blur: targetNumber($event) })"
              />
              <span class="ce-val num">{{ imageCfg.blur }}px</span>
            </div>
          </div>
        </template>
        <input
          ref="imageInput"
          type="file"
          accept="image/png,image/jpeg,image/webp"
          class="hidden"
          @change="onImagePick"
        />
      </template>

      <!-- 全局排版：卡片透明度 / 圆角 / 字号 -->
      <div class="ce-row">
        <span class="ce-label">{{ t('settings.themeCardOpacity') }}</span>
        <div class="ce-slider-row">
          <input
            type="range"
            class="ce-range"
            min="20"
            max="100"
            :value="cardOpacityPct"
            @input="cardOpacityPct = targetNumber($event)"
          />
          <span class="ce-val num">{{ cardOpacityPct }}%</span>
        </div>
      </div>
      <div class="ce-row">
        <span class="ce-label">{{ t('settings.themeRadius') }}</span>
        <div class="ce-slider-row">
          <input
            type="range"
            class="ce-range"
            min="0"
            max="16"
            :value="radiusPx"
            @input="radiusPx = targetNumber($event)"
          />
          <span class="ce-val num">{{ radiusPx }}px</span>
        </div>
      </div>
      <div class="ce-row">
        <span class="ce-label">{{ t('settings.themeFontSize') }}</span>
        <div class="ce-slider-row">
          <input
            type="range"
            class="ce-range"
            min="85"
            max="130"
            step="5"
            :value="fontScalePct"
            @input="fontScalePct = targetNumber($event)"
          />
          <span class="ce-val num">{{ fontScalePct }}%</span>
        </div>
      </div>

      <div class="mt-3 flex justify-end">
        <button type="button" class="miyo-btn miyo-btn-sm" @click="theme.resetBackground()">
          {{ t('settings.themeBgReset') }}
        </button>
      </div>
    </div>

    <!-- 底部操作：导入 / 导出 / 恢复默认主题 -->
    <div class="mt-4 flex items-center gap-2 border-t border-line pt-4">
      <button type="button" class="miyo-btn miyo-btn-sm" @click="importInput?.click()">
        <Icon name="upload" :size="12" aria-hidden="true" />
        {{ t('settings.themeImport') }}
      </button>
      <button type="button" class="miyo-btn miyo-btn-sm" @click="exportTheme">
        <Icon name="download" :size="12" aria-hidden="true" />
        {{ t('settings.themeExport') }}
      </button>
      <span class="flex-1" />
      <button type="button" class="miyo-btn miyo-btn-sm miyo-btn-ghost" @click="resetTheme">
        {{ t('settings.themeResetAll') }}
      </button>
    </div>
    <input ref="importInput" type="file" accept=".json,application/json" class="hidden" @change="onImportPick" />
  </div>
</template>

<style scoped>
/* 编辑器行：标签列 + 控件列，统一 32px 行距 */
.ce-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 5px 0;
}
.ce-label {
  flex-shrink: 0;
  font-size: var(--fs-xs);
  color: var(--text-2);
}
.ce-color {
  display: inline-flex;
  align-items: center;
  gap: 8px;
}
.ce-color-input {
  width: 34px;
  height: 26px;
  padding: 0;
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-sm);
  background: transparent;
  cursor: pointer;
}
.ce-color-input::-webkit-color-swatch-wrapper {
  padding: 2px;
}
.ce-color-input::-webkit-color-swatch {
  border: none;
  border-radius: calc(var(--radius-sm) - 1px);
}
.ce-hex {
  min-width: 62px;
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  color: var(--text-3);
  text-transform: uppercase;
}
.ce-mini-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-3);
  cursor: pointer;
}
.ce-mini-btn:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}
.ce-slider-row {
  display: inline-flex;
  align-items: center;
  gap: 8px;
}
.ce-range {
  width: 180px;
  accent-color: var(--accent);
  cursor: pointer;
}
.ce-val {
  min-width: 44px;
  text-align: right;
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  color: var(--text-3);
}
.ce-preview {
  height: 36px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
}
.ce-thumb {
  width: 44px;
  height: 30px;
  object-fit: cover;
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-sm);
}
</style>
