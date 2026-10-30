<script setup lang="ts">
/**
 * ScriptsPanel：Pre-request / Post-response 脚本编辑（Postman 风格 pm.* API）。
 * 脚本随接口规格持久化（request.scripts），发送时由后端沙箱执行：
 * pre 在变量渲染后、发送前运行（可改头/读写变量），post 在响应到达后运行
 * （pm.test 断言与 console 输出随响应返回，见 ResponsePanel「脚本」页签）。
 */
import { computed } from 'vue'
import { useLocaleStore } from '../stores/locale'
import type { Endpoint } from '../types/miyoApi'

const props = defineProps<{ draft: Endpoint }>()
const t = useLocaleStore().t

/** scripts 对象按需创建（旧接口无此字段，未编辑前保持缺省，避免无谓落库）。
 *  经局部别名赋值（与其他面板同法，规避 vue/no-mutating-props 对别名链的误报）。 */
function ensureScripts(): NonNullable<Endpoint['request']['scripts']> {
  const req = props.draft.request
  if (!req.scripts) {
    req.scripts = { pre_request: '', post_response: '' }
  }
  return req.scripts
}

const preRequest = computed({
  get: () => props.draft.request.scripts?.pre_request ?? '',
  set: (v: string) => {
    ensureScripts().pre_request = v
  },
})

const postResponse = computed({
  get: () => props.draft.request.scripts?.post_response ?? '',
  set: (v: string) => {
    ensureScripts().post_response = v
  },
})
</script>

<template>
  <div class="scripts-panel">
    <section class="script-section">
      <h3 class="script-title">{{ t('editor.scriptsPreTitle') }}</h3>
      <textarea
        v-model="preRequest"
        class="script-editor"
        spellcheck="false"
        :placeholder="t('editor.scriptsPrePh')"
        aria-label="pre-request script"
      ></textarea>
    </section>
    <section class="script-section">
      <h3 class="script-title">{{ t('editor.scriptsPostTitle') }}</h3>
      <textarea
        v-model="postResponse"
        class="script-editor"
        spellcheck="false"
        :placeholder="t('editor.scriptsPostPh')"
        aria-label="post-response script"
      ></textarea>
    </section>
    <p class="script-hint">{{ t('editor.scriptsHint') }}</p>
  </div>
</template>

<style scoped>
.scripts-panel {
  display: flex;
  flex-direction: column;
  gap: 10px;
  height: 100%;
  min-height: 0;
  overflow: auto;
  padding: 10px 12px;
}

.script-section {
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-height: 0;
  flex: 1;
}

.script-title {
  margin: 0;
  font-size: 12px;
  font-weight: 600;
  color: var(--text-2);
}

.script-editor {
  flex: 1;
  min-height: 120px;
  resize: vertical;
  padding: 8px 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-code);
  color: var(--text-1);
  font-family: var(--font-mono);
  font-size: 12px;
  line-height: 1.6;
  outline: none;
}

.script-editor:focus {
  border-color: var(--accent, #a78bfa);
}

.script-hint {
  margin: 0;
  font-size: var(--fs-xxs);
  line-height: 1.6;
  color: var(--text-3);
}
</style>
