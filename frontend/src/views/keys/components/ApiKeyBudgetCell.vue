<script setup lang="ts">
import type { ApiKey } from '@/api'
import BudgetCell from '@/components/BudgetCell.vue'

defineProps<{ apiKey: ApiKey }>()
</script>

<template>
  <BudgetCell :budget="apiKey" :name="apiKey.name" />
</template>
