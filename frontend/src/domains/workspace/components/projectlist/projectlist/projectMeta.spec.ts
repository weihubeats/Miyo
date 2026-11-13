/**
 * projectMeta 单测：头像缩写（中英文） / 配色稳定 / 相对时间。
 */
import { beforeEach, describe, expect, it } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { avatarStyle, initials, timeAgo } from '@/domains/workspace/components/projectlist/projectlist/projectMeta';
import { useLocaleStore } from '@/stores/locale';

beforeEach(() => {
  setActivePinia(createPinia());
  // 文案断言锁定中文（jsdom 默认语言为英文，跟随系统会解析出英文）
  useLocaleStore().setMode('zh');
});

describe('initials', () => {
  it('英文多词取首字母', () => {
    expect(initials('Demo API')).toBe('DA');
  });

  it('中文同前缀项目取末段前两字（有区分度）', () => {
    expect(initials('小奏技术·用户服务')).toBe('用户');
    expect(initials('小奏技术·开放演示')).toBe('开放');
    expect(initials('小奏技术·GraphQL 网关')).not.toBe(initials('小奏技术·用户服务'));
  });

  it('无分隔符时取前两字；空名兜底', () => {
    expect(initials('演示')).toBe('演示');
    expect(initials('  ')).toBe('?');
  });
});

describe('avatarStyle', () => {
  it('同名稳定、异名大概率不同', () => {
    expect(avatarStyle('A')).toEqual(avatarStyle('A'));
    expect(avatarStyle('A')).not.toEqual(avatarStyle('B'));
  });
});

describe('timeAgo', () => {
  it('刚刚 / 分钟前 / 小时前', () => {
    const now = Date.now();
    expect(timeAgo(new Date(now - 10_000).toISOString())).toContain('刚刚');
    expect(timeAgo(new Date(now - 5 * 60_000).toISOString())).toContain('5');
    expect(timeAgo(new Date(now - 3 * 3600_000).toISOString())).toContain('3');
  });
});
