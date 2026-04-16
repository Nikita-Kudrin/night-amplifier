<script setup>
defineProps({
  title: {
    type: String,
    default: '',
  },
  bordered: {
    type: Boolean,
    default: true,
  },
})
</script>

<template>
  <div class="panel" :class="{ 'panel-bordered': bordered }">
    <div v-if="title || $slots.header" class="panel-header">
      <h2 v-if="title">{{ title }}</h2>
      <slot name="header"/>
    </div>
    <slot/>
  </div>
</template>

<style scoped>
/* Uses global .panel, .panel-bordered, .panel-header from main.css */
</style>
