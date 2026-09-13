<script setup lang="ts">
defineProps<{
  maxConcurrency: number
  requestsPerMinute: number
  currentConcurrency?: number | null
  currentRpm?: number | null
  size?: 'sm' | 'md'
}>()
</script>

<template>
  <dl class="m-0 grid grid-cols-[auto_minmax(0,1fr)] gap-x-2 gap-y-1 tabular-nums" :class="size === 'md' ? 'text-cp-sm' : 'text-xs'">
    <dt class="text-cp-text-tertiary" title="当前占用执行名额的请求数 / 并发限制">
      并发
    </dt>
    <dd class="m-0 truncate text-cp-text" :title="`${currentConcurrency ?? '—'} / ${maxConcurrency || '∞'}`">
      {{ currentConcurrency ?? '—' }} / {{ maxConcurrency || '∞' }}
    </dd>
    <dt class="text-cp-text-tertiary" title="最近滚动 60 秒内取得执行名额的请求数 / RPM 限制">
      RPM
    </dt>
    <dd class="m-0 truncate text-cp-text" :title="`${currentRpm ?? '—'} / ${requestsPerMinute || '∞'}`">
      {{ currentRpm ?? '—' }} / {{ requestsPerMinute || '∞' }}
    </dd>
  </dl>
</template>
