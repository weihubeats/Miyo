<script setup lang="ts">
/**
 * JavaSetupGuide：Java Debug SDK 集成教程弹窗。
 *
 * 三种引入方式（Maven / Gradle / 拷贝源码）Tab + Spring Boot / 普通 Java
 * 代码示例 + 启动验证命令 + 注册文件说明。代码片段全部存于脚本常量经
 * 插值渲染（不放模板静态文本节点）；复制按钮走剪贴板。
 */
import { computed, ref } from 'vue'
import Icon from '@/components/ui/Icon.vue'
import Modal from '@/components/ui/Modal.vue'
import SegmentedControl, { type SegmentOption } from '@/components/ui/SegmentedControl.vue'
import { useToast } from '@/composables/useToast'
import { useLocaleStore } from '@/stores/locale'

const props = defineProps<{ open: boolean }>()
const emit = defineEmits<{ close: [] }>()

const toast = useToast()
const locale = useLocaleStore()
const t = locale.t

// ---- 代码片段（纯示例内容，非 UI 文案） ----
const MVN_INSTALL = `cd java/miyo-java-runtime && mvn install
# io.miyo:miyo-java-runtime:0.1.0`

const MAVEN_DEP = `<dependency>
  <groupId>io.miyo</groupId>
  <artifactId>miyo-java-runtime</artifactId>
  <version>0.1.0</version>
</dependency>`

const GRADLE_DEP = `implementation 'io.miyo:miyo-java-runtime:0.1.0'`

const SPRING_CODE = `@Component
public class MiyoDebugStarter implements ApplicationRunner, DisposableBean {

    private final ApplicationContext ctx;
    private MiyoDebugRuntime runtime;

    public MiyoDebugStarter(ApplicationContext ctx) { this.ctx = ctx; }

    @Override
    public void run(ApplicationArguments args) throws Exception {
        runtime = MiyoDebugRuntime.builder(System.getenv("MIYO_DEBUG_TOKEN"))
                .appName("order-service")
                .port(9918)
                .instanceProvider("spring")
                .register("orderService", OrderService.class,
                        () -> ctx.getBean(OrderService.class))   // 经容器取 Bean
                .register(null, OrderDomain.class,
                        InstanceProvider.staticInstance())       // 静态工具类
                .start();
    }

    @Override
    public void destroy() {
        if (runtime != null) runtime.stop();
    }
}`

const PLAIN_CODE = `public class Main {
    public static void main(String[] args) throws Exception {
        MiyoDebugRuntime runtime = MiyoDebugRuntime.builder(
                        System.getenv("MIYO_DEBUG_TOKEN"))
                .appName("plain-app")
                .port(9918)
                .register(null, MathUtil.class, InstanceProvider.staticInstance())
                .registerInstance("paymentService", new PaymentService())
                .start();

        // 应用退出钩子（至少清理注册文件）
        Runtime.getRuntime().addShutdownHook(new Thread(runtime::stop));
    }
}`

const VERIFY_CMD = `curl -H "Authorization: Bearer $MIYO_DEBUG_TOKEN" \\
     -H "X-Miyo-Protocol-Version: 1" \\
     http://127.0.0.1:9918/miyo/debug/info`

// ---- Tab（随语言刷新：computed 包裹） ----
const modeOptions = computed<SegmentOption[]>(() => [
  { value: 'maven', label: t('java.guideTabMaven') },
  { value: 'gradle', label: t('java.guideTabGradle') },
  { value: 'source', label: t('java.guideTabSource') },
])
const mode = ref<'maven' | 'gradle' | 'source'>('maven')

const depSnippet = computed(() => {
  if (mode.value === 'maven') return MAVEN_DEP
  if (mode.value === 'gradle') return GRADLE_DEP
  return 'io/miyo/debug/*.java → src/main/java/io/miyo/debug/'
})
const depLabel = computed(() => {
  if (mode.value === 'maven') return t('java.guideMavenLabel')
  if (mode.value === 'gradle') return t('java.guideGradleLabel')
  return t('java.guideSourceLabel')
})

async function copySnippet(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text)
    toast.success(t('java.guideCopied'))
  } catch {
    toast.error(t('java.copyFailed'))
  }
}
</script>

<template>
  <Modal :open="props.open" :title="t('java.guideTitle')" width="720px" @close="emit('close')">
    <div class="jsg">
      <p class="jsg-intro">{{ t('java.guideIntro') }}</p>

      <!-- 引入方式 -->
      <section class="jsg-block">
        <div class="jsg-block-head">
          <SegmentedControl :model-value="mode" :options="modeOptions" size="sm" @update:model-value="mode = $event as typeof mode" />
          <button type="button" class="miyo-btn jsg-copy" @click="copySnippet(depSnippet)">
            <Icon name="copy" :size="12" />
            {{ t('java.guideCopy') }}
          </button>
        </div>
        <p class="jsg-label">{{ depLabel }}</p>
        <template v-if="mode !== 'source'">
          <pre class="jsg-code">{{ mode === 'maven' ? MVN_INSTALL + '\n\n' + MAVEN_DEP : GRADLE_DEP }}</pre>
        </template>
        <pre v-else class="jsg-code">{{ depSnippet }}</pre>
      </section>

      <!-- Spring Boot -->
      <section class="jsg-block">
        <div class="jsg-block-head">
          <h3 class="jsg-sub">{{ t('java.guideSpringTitle') }}</h3>
          <button type="button" class="miyo-btn jsg-copy" @click="copySnippet(SPRING_CODE)">
            <Icon name="copy" :size="12" />
            {{ t('java.guideCopy') }}
          </button>
        </div>
        <p class="jsg-label">{{ t('java.guideSpringHint') }}</p>
        <pre class="jsg-code">{{ SPRING_CODE }}</pre>
      </section>

      <!-- 普通 Java -->
      <section class="jsg-block">
        <div class="jsg-block-head">
          <h3 class="jsg-sub">{{ t('java.guidePlainTitle') }}</h3>
          <button type="button" class="miyo-btn jsg-copy" @click="copySnippet(PLAIN_CODE)">
            <Icon name="copy" :size="12" />
            {{ t('java.guideCopy') }}
          </button>
        </div>
        <p class="jsg-label">{{ t('java.guidePlainHint') }}</p>
        <pre class="jsg-code">{{ PLAIN_CODE }}</pre>
      </section>

      <!-- 启动验证 -->
      <section class="jsg-block">
        <div class="jsg-block-head">
          <h3 class="jsg-sub">{{ t('java.guideVerifyTitle') }}</h3>
          <button type="button" class="miyo-btn jsg-copy" @click="copySnippet(VERIFY_CMD)">
            <Icon name="copy" :size="12" />
            {{ t('java.guideCopy') }}
          </button>
        </div>
        <p class="jsg-label">{{ t('java.guideVerifyDesc') }}</p>
        <pre class="jsg-code">{{ VERIFY_CMD }}</pre>
      </section>

      <p class="jsg-note">{{ t('java.guideRegistryHint') }}</p>
    </div>
  </Modal>
</template>

<style scoped>
.jsg {
  display: flex;
  flex-direction: column;
  gap: 14px;
  max-height: 62vh;
  overflow-y: auto;
  padding-right: 2px;
}
.jsg-intro {
  margin: 0;
  font-size: 12.5px;
  color: var(--text-secondary);
}
.jsg-block {
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  padding: 10px 12px;
  background: var(--bg-elevated);
}
.jsg-block-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  margin-bottom: 6px;
}
.jsg-sub {
  margin: 0;
  font-size: 12.5px;
  font-weight: 600;
  color: var(--text);
}
.jsg-copy {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: 11px;
  padding: 3px 8px;
}
.jsg-label {
  margin: 0 0 6px;
  font-size: 11.5px;
  color: var(--text-secondary);
}
.jsg-code {
  margin: 0;
  padding: 10px;
  border-radius: var(--radius-md);
  background: var(--bg-hover);
  border: 1px solid var(--border);
  font-family: var(--font-mono, monospace);
  font-size: 11px;
  line-height: 1.6;
  overflow-x: auto;
  white-space: pre;
  color: var(--text);
}
.jsg-note {
  margin: 0;
  font-size: 11.5px;
  color: var(--text-tertiary, var(--text-secondary));
  line-height: 1.6;
}
</style>
