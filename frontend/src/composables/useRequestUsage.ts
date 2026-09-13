import type { MaybeRefOrGetter } from 'vue'
import type { RequestUsage, RequestUsageScope } from '@/api/modules/request-usage'
import { useDocumentVisibility, useIntervalFn } from '@vueuse/core'
import { computed, onScopeDispose, shallowRef, toValue, watch } from 'vue'
import { getRequestUsage } from '@/api/modules/request-usage'

export function useRequestUsage(scope: RequestUsageScope, source: MaybeRefOrGetter<readonly string[]>) {
  const ids = computed(() => [...new Set(toValue(source))])
  const visibility = useDocumentVisibility()
  const usage = shallowRef(new Map<string, RequestUsage>())
  let controller: AbortController | undefined
  let sequence = 0
  let running = false

  async function refresh() {
    if (running || !ids.value.length || visibility.value === 'hidden')
      return
    running = true
    const current = ++sequence
    controller = new AbortController()
    const options = { signal: controller.signal, silent: true }
    try {
      const batches = []
      for (let offset = 0; offset < ids.value.length; offset += 100)
        batches.push(getRequestUsage(scope, ids.value.slice(offset, offset + 100), options))
      const result = await Promise.all(batches)
      if (current === sequence)
        usage.value = new Map(result.flat().map(item => [item.id, item]))
    }
    catch {
      if (current === sequence)
        usage.value = new Map()
    }
    finally {
      if (current === sequence)
        running = false
    }
  }

  const { pause, resume } = useIntervalFn(() => void refresh(), 5000, { immediate: false })
  watch([ids, visibility], () => {
    sequence++
    controller?.abort()
    running = false
    pause()
    usage.value = new Map()
    if (visibility.value !== 'hidden' && ids.value.length) {
      void refresh()
      resume()
    }
  }, { immediate: true })
  onScopeDispose(() => {
    sequence++
    controller?.abort()
    pause()
  })
  return usage
}
