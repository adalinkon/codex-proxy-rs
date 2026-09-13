<script setup lang="ts">
import type {
  getUsageRecordInsightsDiagnostics,
  getUsageRecordInsightsOverview,
} from '@/api'

import UsageCostCard from './UsageCostCard.vue'
import UsageDiagnosticCard from './UsageDiagnosticCard.vue'
import UsageHealthCard from './UsageHealthCard.vue'
import UsagePerformanceCard from './UsagePerformanceCard.vue'

withDefaults(
  defineProps<{
    overview: Awaited<ReturnType<typeof getUsageRecordInsightsOverview>>
    diagnostics: Awaited<ReturnType<typeof getUsageRecordInsightsDiagnostics>>
    loading?: boolean
    personal?: boolean
  }>(),
  {
    loading: false,
  },
)

const diagnosticDimension = defineModel('diagnosticDimension', {
  type: String,
  default: 'model',
})
</script>

<template>
  <section
    class="mt-5 grid grid-cols-1 gap-3 xl:auto-rows-97"
    :class="{ 'xl:grid-cols-2': !personal }"
    aria-label="使用统计观测"
  >
    <UsageHealthCard
      v-if="!personal"
      :health="overview.health"
      :granularity="overview.granularity"
      :loading="loading"
    />

    <UsageDiagnosticCard
      v-if="!personal"
      v-model:dimension="diagnosticDimension"
      :personal="personal"
      :diagnostics="diagnostics"
      :loading="loading"
    />

    <UsagePerformanceCard
      v-if="!personal"
      :performance="overview.performance"
      :activity="overview.health.points"
      :loading="loading"
    />

    <UsageCostCard
      :cost="overview.cost"
      :activity="overview.health.points"
      :loading="loading"
    />
  </section>
</template>
