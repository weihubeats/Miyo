/**
 * lazyComponent：低频 / 重型面板的按需加载封装（defineAsyncComponent 工厂）。
 *
 * - loading：沿用 Vue 默认 200ms 延迟才挂骨架，本地 chunk 秒开时不闪占位；
 * - error：失败展示可翻译的错误层 + 重试按钮（重试入口经 WeakMap 由 onError 注入，
 *   errorComponent 只拿得到 error prop）；
 * - 自动重试一次：WebView 偶发的 chunk 拉取抖动不该直接把用户拍在错误页。
 *
 * 用法：`const Foo = lazyComponent(() => import('@/composables/Foo.vue'))`。
 * 仅用于「打开才出现」或常驻但不在首屏关键路径的面板；首屏骨架 / 响应区保持同步，
 * 否则会出现一帧占位或测试需额外等待。
 */
import { defineAsyncComponent, h, type AsyncComponentLoader, type Component } from 'vue'
import Skeleton from '@/components/ui/Skeleton.vue'
import { useLocaleStore } from '@/stores/locale'

/** 错误对象 → 重试函数：错误层只能拿到 error prop，用它反查 retry 通道。 */
const retryByError = new WeakMap<Error, () => void>()

/** 加载占位：骨架 + 无障碍 busy 标记，与响应区同款视觉。 */
const Loading: Component = {
  name: 'LazyPanelLoading',
  setup() {
    return () =>
      h(
        'div',
        { class: 'miyo-lazy-loading', 'aria-busy': 'true', role: 'status' },
        h(Skeleton, { lines: 3 }),
      )
  },
}

/** 失败占位：文案与重试按钮均走 t()（见 stores/locale.ts 入口约定）。 */
const LoadError: Component = {
  name: 'LazyPanelError',
  props: { error: { type: Error, default: null } },
  setup(props) {
    const t = useLocaleStore().t
    return () =>
      h('div', { class: 'miyo-lazy-error', role: 'alert' }, [
        h('span', { class: 'miyo-lazy-error-text' }, t('common.panelLoadFail')),
        h(
          'button',
          {
            type: 'button',
            class: 'miyo-btn miyo-btn-sm',
            onClick: () => {
              if (props.error) retryByError.get(props.error)?.()
            },
          },
          t('common.retry'),
        ),
      ])
  },
}

/** 把组件加载器包装成带 loading / error 层的异步组件。 */
export function lazyComponent(loader: AsyncComponentLoader): Component {
  return defineAsyncComponent({
    loader,
    loadingComponent: Loading,
    errorComponent: LoadError,
    onError: (error, retry, fail, attempts) => {
      retryByError.set(error, () => retry())
      if (attempts < 2) retry()
      else fail()
    },
  })
}
