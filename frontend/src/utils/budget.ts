export function formatBudgetAmount(value: string) {
  // 展示保留两位小数，完整十进制值继续用于精度提示。
  return Number(value).toLocaleString('en-US', { maximumFractionDigits: 2 })
}

export function formatBudgetResetCountdown(resetAt: string | null, period: '日' | '周', now: number) {
  const reset = resetAt ? Date.parse(resetAt) : Number.NaN
  if (!Number.isFinite(reset))
    return '重置时间暂不可用'
  const remaining = reset - now
  if (remaining <= 0)
    return '已到重置时间'
  // 向上取最小展示单位，避免尚未到期就显示零分或零小时。
  if (period === '日') {
    const minutes = Math.ceil(remaining / 60_000)
    return `${Math.floor(minutes / 60)}小时${minutes % 60}分后重置`
  }
  const hours = Math.ceil(remaining / 3_600_000)
  return `${Math.floor(hours / 24)}天${hours % 24}小时后重置`
}
