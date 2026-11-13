/**
 * 项目列表展示相关的纯工具函数：
 * 头像配色 / 名称缩写 / 相对时间。
 */
import { tFallback } from '@/stores/locale'

/** 头像配色盘（按名称哈希取色，保证同名项目颜色稳定） */
export const PALETTE = [
  { bg: 'rgba(124, 105, 245, 0.16)', color: '#a78bfa' },
  { bg: 'rgba(34, 197, 94, 0.14)', color: '#34d399' },
  { bg: 'rgba(59, 130, 246, 0.14)', color: '#60a5fa' },
  { bg: 'rgba(245, 158, 11, 0.14)', color: '#fbbf24' },
  { bg: 'rgba(236, 72, 153, 0.14)', color: '#f472b6' },
  { bg: 'rgba(6, 182, 212, 0.14)', color: '#22d3ee' },
]

export function avatarStyle(name: string): { background: string; color: string } {
  let h = 0
  for (let i = 0; i < name.length; i++) h = (h * 31 + name.charCodeAt(i)) >>> 0
  const c1 = PALETTE[h % PALETTE.length]
  const c2 = PALETTE[(h * 2654435761) % PALETTE.length]
  return {
    background: `linear-gradient(135deg, ${c1.color}2e 0%, ${c2.color}2e 100%)`,
    color: c1.color,
  }
}

export function initials(name: string): string {
  const trimmed = name.trim();
  if (!trimmed) return '?';
  const parts = trimmed.split(/\s+/).filter(Boolean);
  if (parts.length >= 2) return (parts[0][0] + parts[1][0]).toUpperCase();
  // 中文名常带共同前缀（如「小奏技术·用户服务」）：取末段前两字才有区分度
  const segments = trimmed.split(/[·•・:：\-—_\/|｜]/).map((s) => s.trim()).filter(Boolean);
  const last = segments.length > 1 ? segments[segments.length - 1] : trimmed;
  return last.slice(0, 2).toUpperCase() || '?';
}

export function timeAgo(iso: string): string {
  const diff = Date.now() - new Date(iso).getTime()
  const min = Math.floor(diff / 60000)
  if (min < 1) return tFallback('projectlist.timeJustNow')
  if (min < 60) return tFallback('projectlist.timeMinAgo', { n: min })
  const hours = Math.floor(min / 60)
  if (hours < 24) return tFallback('projectlist.timeHourAgo', { n: hours })
  const days = Math.floor(hours / 24)
  if (days < 30) return tFallback('projectlist.timeDayAgo', { n: days })
  return iso.slice(0, 10)
}
