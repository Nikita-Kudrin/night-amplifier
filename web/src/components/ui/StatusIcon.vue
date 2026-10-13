<script>
/** The status bar's 14 px line icons, by name. */
export const STATUS_ICON_SHAPES = {
  camera: [
    ['path', {d: 'M23 19a2 2 0 01-2 2H3a2 2 0 01-2-2V8a2 2 0 012-2h4l2-3h6l2 3h4a2 2 0 012 2z'}],
    ['circle', {cx: 12, cy: 13, r: 4}],
  ],
  check: [['path', {d: 'M20 6L9 17l-5-5'}]],
  stop: [['circle', {cx: 12, cy: 12, r: 10}], ['path', {d: 'M9 9h6v6H9z'}]],
  cross: [['circle', {cx: 12, cy: 12, r: 10}], ['path', {d: 'M15 9l-6 6M9 9l6 6'}]],
  alert: [['circle', {cx: 12, cy: 12, r: 10}], ['path', {d: 'M12 8v4M12 16h.01'}]],
  blocked: [['circle', {cx: 12, cy: 12, r: 10}], ['path', {d: 'M12 8v5M12 16h.01'}]],
  info: [['circle', {cx: 12, cy: 12, r: 10}], ['path', {d: 'M12 16v-4'}], ['path', {d: 'M12 8h.01'}]],
  warning: [
    ['path', {d: 'M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z'}],
    ['path', {d: 'M12 9v4M12 17h.01'}],
  ],
  resume: [['path', {d: 'M20 12a8 8 0 11-2.34-5.66'}], ['path', {d: 'M20 4v4h-4'}]],
  frames: [
    ['rect', {x: 3, y: 3, width: 18, height: 18, rx: 2, ry: 2}],
    ['circle', {cx: 8.5, cy: 8.5, r: 1.5}],
    ['polyline', {points: '21,15 16,10 5,21'}],
  ],
  dropped: [['path', {d: 'M12 2v6l-2 2M12 22v-6l2-2M2 12h6l2 2M22 12h-6l-2-2'}]],
}
</script>

<script setup>
defineProps({
  name: {type: String, required: true, validator: (name) => name in STATUS_ICON_SHAPES},
})
</script>

<template>
  <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true">
    <component :is="tag" v-for="([tag, attrs], i) in STATUS_ICON_SHAPES[name]" :key="i" v-bind="attrs"/>
  </svg>
</template>
