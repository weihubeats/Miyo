import { setActivePinia, createPinia } from 'pinia'
import { mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it } from 'vitest'
import { useLocaleStore } from '@/stores/locale'
import type { JavaClassCategory, JavaClassInfo } from '@/types/miyoApi.d'
import JavaClassBrowser from './components/JavaClassBrowser.vue'
import { CATEGORY_ORDER, ROLE_ORDER, groupClasses, statsSummary } from './utils/classGroups'

function row(
  name: string,
  category: JavaClassCategory,
  role?: JavaClassInfo['role'],
): JavaClassInfo {
  const dot = name.lastIndexOf('.')
  return {
    class_name: name,
    package_name: dot >= 0 ? name.slice(0, dot) : '',
    simple_name: dot >= 0 ? name.slice(dot + 1) : name,
    method_count: -1,
    bean_name: null,
    category,
    role,
  }
}

describe('classGroups', () => {
  it('按类别顺序分组，业务类再按角色细分', () => {
    const groups = groupClasses([
      row('com.a.OrderService', 'application', 'service'),
      row('com.a.OrderController', 'application', 'controller'),
      row('com.a.OrderHelper', 'application', 'other'),
      row('com.a.PricingService', 'spring_bean', 'service'),
      row('io.netty.EventLoop', 'dependency'),
      row('java.util.List', 'jdk_internal'),
    ])
    expect(groups.map((g) => g.key)).toEqual([
      'application:controller',
      'application:service',
      'application:other',
      'spring_bean:service',
      'dependency',
      'jdk_internal',
    ])
    expect(groups.map((g) => g.count)).toEqual([1, 1, 1, 1, 1, 1])
  })

  it('依赖 / JDK / 生成类默认折叠，业务组默认展开', () => {
    const groups = groupClasses([
      row('com.a.Svc', 'application', 'service'),
      row('io.netty.EventLoop', 'dependency'),
      row('java.util.List', 'jdk_internal'),
      row('com.a.Svc$$EnhancerBySpringCGLIB$$1', 'generated'),
    ])
    const collapsed = Object.fromEntries(groups.map((g) => [g.key, g.collapsed]))
    expect(collapsed['application:service']).toBe(false)
    expect(collapsed.dependency).toBe(true)
    expect(collapsed.jdk_internal).toBe(true)
    expect(collapsed.generated).toBe(true)
  })

  it('未知类归入未识别，不混入业务类', () => {
    const groups = groupClasses([row('mystery.Thing', 'unknown')])
    expect(groups).toHaveLength(1)
    expect(groups[0].category).toBe('unknown')
  })

  it('收藏项在所属组内置顶', () => {
    const groups = groupClasses(
      [
        row('com.a.AlphaService', 'application', 'service'),
        row('com.a.ZuluService', 'application', 'service'),
      ],
      (name) => name === 'com.a.ZuluService',
    )
    expect(groups[0].items.map((i) => i.simple_name)).toEqual(['ZuluService', 'AlphaService'])
  })

  it('顺序常量覆盖全部类别与角色', () => {
    expect(CATEGORY_ORDER[0]).toBe('application')
    expect(ROLE_ORDER[0]).toBe('controller')
    expect(statsSummary({ application: 3, dependency: 10 })).toBe('application:3 dependency:10')
    expect(statsSummary(null)).toBe('')
  })
})

describe('JavaClassBrowser（分类树）', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    useLocaleStore().setMode('zh')
  })

  function mountBrowser(classes: JavaClassInfo[]) {
    return mount(JavaClassBrowser, {
      props: {
        classes,
        loading: false,
        connected: true,
        activeClassName: null,
        search: '',
        recent: [],
        isFavorite: () => false,
        stats: {
          total: classes.length,
          loaded_class_count: classes.length,
          by_category: { application: 1, dependency: 1 },
          by_role: {},
          generation: 1,
          indexed_at: 0,
        },
      },
    })
  }

  it('只有业务类（无 Bean）时默认 grouped 推断仍成立', () => {
    const wrapper = mountBrowser([
      row('com.a.OrderService', 'application', 'service'),
      row('io.netty.EventLoop', 'dependency'),
    ])
    expect(wrapper.find('.jcb-group-head').exists()).toBe(true)
  })

  it('分组标题按中文类别渲染，依赖组默认折叠', async () => {
    const wrapper = mountBrowser([
      row('com.a.OrderService', 'application', 'service'),
      row('io.netty.EventLoop', 'dependency'),
    ])
    const text = wrapper.text()
    expect(text).toContain('Service')
    expect(text).toContain('第三方依赖')
    // 折叠的依赖组不渲染条目（注意：扁平分支里 OrderService 后紧跟 EventLoop）
    expect(wrapper.text()).not.toContain('EventLoop')

    const heads = wrapper.findAll('.jcb-group-head')
    await heads[1].trigger('click')
    expect(wrapper.text()).toContain('EventLoop')
  })

  it('点击类项发出 select 事件', async () => {
    const wrapper = mountBrowser([row('com.a.OrderService', 'application', 'service')])
    await wrapper.find('.jcb-item').trigger('click')
    expect(wrapper.emitted('select')?.[0]).toEqual(['com.a.OrderService'])
  })

  /**
   * 回归（真实用户截图）：attach 模式首屏还没有 Spring Bean 时，分类视图
   * 也必须成立——组标题 / 类别 tab / 开关常驻，不能退回显示「-1 个方法」的
   * 扁平旧列表。
   */
  it('显式 categorized 时即使只有业务类也渲染分类树', () => {
    const wrapper = mount(JavaClassBrowser, {
      props: {
        classes: [row('com.example.PriceService', 'application', 'service')],
        loading: false,
        connected: true,
        activeClassName: null,
        search: '',
        recent: [],
        isFavorite: () => false,
        categorized: true,
      },
    })
    expect(wrapper.find('.jcb-group-head').exists()).toBe(true)
    expect(wrapper.find('.jcb-cats').exists()).toBe(true)
    expect(wrapper.text()).toContain('业务代码')
    // 不再出现扁平分支的「-1 个方法」
    expect(wrapper.text()).not.toContain('-1')
  })

  it('无分类字段（runtime 模式）退回扁平列表', () => {
    const wrapper = mount(JavaClassBrowser, {
      props: {
        classes: [
          {
            class_name: 'com.a.Plain',
            package_name: 'com.a',
            simple_name: 'Plain',
            method_count: 3,
            bean_name: null,
          },
        ],
        loading: false,
        connected: true,
        activeClassName: null,
        search: '',
        recent: [],
        isFavorite: () => false,
      },
    })
    expect(wrapper.find('.jcb-group').exists()).toBe(false)
    expect(wrapper.findAll('.jcb-item')).toHaveLength(1)
  })
})