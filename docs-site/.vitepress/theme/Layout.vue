<script setup lang="ts">
import DefaultTheme from 'vitepress/theme'
import FloatingSupport from './FloatingSupport.vue'
</script>

<template>
  <DefaultTheme.Layout>
    <template #layout-bottom>
      <FloatingSupport />
    </template>
  </DefaultTheme.Layout>
</template>
