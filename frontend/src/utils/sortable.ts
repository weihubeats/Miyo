import type Sortable from 'sortablejs';

/** 拖动换位动画的统一时长（项目卡与接口树共用，用户要求两处手感一致）。 */
export const REORDER_ANIMATION_MS = 150;

/** 拖动换位动画的统一缓动曲线（SortableJS easing + CSS 兜底共用）。 */
export const REORDER_EASING = 'cubic-bezier(0.2, 0, 0, 1)';

/** 拖拽期间给 body 挂的全局类（抓取光标 + 禁选，见 style.css `.dragging-dnd`）。 */
export const DRAGGING_BODY_CLASS = 'dragging-dnd';

/** 禁选标记：拖拽容器上只要带着它，CSS 就整块禁选（见 style.css `[data-drag-lock]`）。 */
export const DRAG_LOCK_ATTR = 'data-drag-lock';

/** 按下到松手的位移不超过该值（px）就算纯点击，而不是拖动手势。 */
export const DRAG_CLICK_SLOP_PX = 4;

/** 给 body 挂上全局拖拽态（幂等）。 */
export function beginDragCursor(): void {
  document.body.classList.add(DRAGGING_BODY_CLASS);
}

/** 摘掉全局拖拽态（幂等；卸载时兜底调用，防止异常退出留下抓取光标）。 */
export function endDragCursor(): void {
  document.body.classList.remove(DRAGGING_BODY_CLASS);
}

/* ------------------------------------------------------------------
 * 拖拽禁选 + 纯点击补发
 *
 * 背景（用户实测，首页项目卡）：按下卡片时若不立刻禁选，WKWebView 会在第一段
 * mousemove 之前就把「按下点 → 指针」划成选区，整个页面的文字被反白。禁选
 * 又不能等 SortableJS 的 onStart（它要等指针累积移动 3px 才触发），必须在
 * 指针按下的捕捉期就加上。
 *
 * 副作用：user-select:none 的元素在 WKWebView / Safari 上**不产生 click**
 * （只有 mousedown / mouseup），而接口树是靠 click 打开接口的。所以这里用
 * mousedown → mouseup 的位移自己判定：
 * - 位移 ≤ 阈值：纯点击——按顺序补发 click 与 dblclick（点击处理照常执行，
 *   双击也不丢），且不在 mouseup 里解锁，紧接着的原生 dblclick 仍在禁选内；
 * - 位移 > 阈值：拖动手势——什么也不补发，交给 SortableJS 的 onEnd 收尾。
 * 解锁统一放在 dblclick 或 400ms 定时器里（双击间隔上限），保证「点两下」
 * 不会让第一次点击的禁选把第二下吃掉。
 * ------------------------------------------------------------------ */

export interface DragGuardOptions {
  /** 拖拽容器：只接管落在它内部、且未被 filter 规则排除（button 等）的按下。 */
  container: HTMLElement;
  /** 纯点击补发时要跳过的事件源（与 Sortable 的 filter 同义，避免按钮被点两遍）。 */
  clickFilter?: string;
}

/**
 * 给 SortableJS 实例挂上「按下即禁选 + 纯点击补发 click/dblclick」。
 *
 * 先 `Sortable.create(...)` 建实例，再调用本函数；返回解绑函数（容器卸载 /
 * 实例销毁时调用，内部会摘掉残留的禁选标记）。
 */
export function bindDragGuards(
  instance: Sortable,
  opts: DragGuardOptions,
): () => void {
  const { container, clickFilter = 'button, input, select, textarea, a' } =
    opts;
  let pressed = false;
  let dragging = false;
  let startX = 0;
  let startY = 0;
  let downTarget: EventTarget | null = null;
  let unlockTimer: number | undefined;

  function unlock(): void {
    window.clearTimeout(unlockTimer);
    unlockTimer = undefined;
    container.removeAttribute(DRAG_LOCK_ATTR);
    endDragCursor();
  }

  function reset(): void {
    pressed = false;
    dragging = false;
    downTarget = null;
    unlock();
  }

  function onDown(evt: Event): void {
    const e = evt as MouseEvent;
    // 只认左键；中键 / 右键菜单不接管
    if (e.button !== 0) return;
    const target = e.target as Element | null;
    // 按钮 / 链接 / 输入框：交给它们自己，不接管（也就不会补发重复点击）
    if (target?.closest?.(clickFilter)) {
      pressed = false;
      return;
    }
    window.clearTimeout(unlockTimer);
    pressed = true;
    dragging = false;
    startX = e.clientX;
    startY = e.clientY;
    downTarget = target;
    // 捕捉期：先于 SortableJS 的 onChoose（冒泡期），指针一动也不会反白
    container.setAttribute(DRAG_LOCK_ATTR, '');
  }

  function onMove(evt: Event): void {
    if (!pressed || dragging) return;
    const e = evt as MouseEvent;
    const moved = Math.max(
      Math.abs(e.clientX - startX),
      Math.abs(e.clientY - startY),
    );
    if (moved > DRAG_CLICK_SLOP_PX) dragging = true;
  }

  /** 补发一次合成事件（user-select:none 下原生事件被 WebKit 吞掉，这里补回来）。 */
  function emit(
    type: 'click' | 'dblclick',
    target: EventTarget,
    e: MouseEvent,
  ): void {
    target.dispatchEvent?.(
      new MouseEvent(type, {
        bubbles: true,
        cancelable: true,
        clientX: e.clientX,
        clientY: e.clientY,
        button: 0,
        detail: type === 'click' ? 1 : 2,
      }),
    );
  }

  function onUp(evt: Event): void {
    if (!pressed) return;
    const wasClick = !dragging;
    const target = downTarget;
    pressed = false;
    if (!wasClick || !target) {
      // 拖动：交给 SortableJS 的 onEnd 收尾（会 reset）
      return;
    }
    const e = evt as MouseEvent;
    // 仍保持禁选：紧接着的原生 dblclick（第二次点击）不能被反白选中
    window.clearTimeout(unlockTimer);
    unlockTimer = window.setTimeout(() => unlock(), 400);
    emit('click', target, e);
    emit('dblclick', target, e);
  }

  container.addEventListener('mousedown', onDown, true);
  container.addEventListener('mousemove', onMove, true);
  container.addEventListener('mouseup', onUp, true);
  // 只接管未被排除的按下：被 filter 掉的按钮本身不在列表里拖，也不该影响 onEnd
  const onInstanceEnd = (): void => reset();
  instance.option('onEnd', onInstanceEnd);

  return () => {
    container.removeEventListener('mousedown', onDown, true);
    container.removeEventListener('mousemove', onMove, true);
    container.removeEventListener('mouseup', onUp, true);
    reset();
  };
}
