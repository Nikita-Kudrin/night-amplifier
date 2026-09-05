<script setup>
import {ref, computed, watch, onMounted, onUnmounted, nextTick, useSlots} from 'vue'

const props = defineProps({
  /**
   * Label of the primary action. Rendered as the button's text unless the default
   * slot replaces it, and used as the accessible name either way — an icon-only
   * button still has to say what it does.
   */
  label: {
    type: String,
    required: true,
  },
  /** Accessible name for the menu trigger. */
  menuLabel: {
    type: String,
    default: 'More options',
  },
  /** Secondary actions, as `{value, label, disabled?}`. */
  options: {
    type: Array,
    default: () => [],
  },
  /** Disables the primary action only; the menu stays reachable. */
  disabled: {
    type: Boolean,
    default: false,
  },
  variant: {
    type: String,
    default: 'primary',
    validator: (v) => ['primary', 'secondary', 'danger'].includes(v),
  },
})

/** `menuToggle` carries the menu's open state, for callers that must react to it. */
const emit = defineEmits(['click', 'select', 'menuToggle'])

const slots = useSlots()

/**
 * A button whose slot replaced the label shows an icon and nothing else, so the
 * label has to come back as a hover tooltip. One rendering its own text does not
 * need a tooltip repeating it.
 */
const mainTitle = computed(() => (slots.default ? props.label : undefined))

const open = ref(false)
const rootRef = ref(null)
const menuRef = ref(null)
const menuStyle = ref({})

// A watcher rather than an emit at each site: three paths close the menu, and one
// of them silently not reporting is how a listener ends up stuck open.
watch(open, (isOpen) => emit('menuToggle', isOpen))

async function toggleMenu(e) {
  e.stopPropagation()
  open.value = !open.value
  if (open.value) {
    await nextTick()
    positionMenu()
  }
}

/**
 * The menu is `fixed` and positioned from the trigger's own rect rather than being an
 * absolutely-positioned child. Its container is a short `overflow-y: auto` scroll region
 * (`.cameras-container`), which would clip an in-flow menu entirely.
 */
function positionMenu() {
  if (!menuRef.value || !rootRef.value) return

  const trigger = rootRef.value.getBoundingClientRect()
  const menu = menuRef.value.getBoundingClientRect()
  const padding = 8

  let left = trigger.right - menu.width
  left = Math.max(padding, Math.min(left, window.innerWidth - menu.width - padding))

  // Drop upward when the space below cannot hold the menu.
  const spaceBelow = window.innerHeight - trigger.bottom
  const dropUp = spaceBelow < menu.height + padding && trigger.top > menu.height + padding
  const top = dropUp ? trigger.top - menu.height - 4 : trigger.bottom + 4

  menuStyle.value = {left: `${left}px`, top: `${top}px`}
}

function choose(option) {
  if (option.disabled) return
  open.value = false
  emit('select', option.value)
}

function handleClickOutside(e) {
  if (open.value && rootRef.value && !rootRef.value.contains(e.target)) {
    if (menuRef.value && menuRef.value.contains(e.target)) return
    open.value = false
  }
}

onMounted(() => {
  document.addEventListener('click', handleClickOutside)
  window.addEventListener('resize', positionMenu)
  window.addEventListener('scroll', positionMenu, true)
})

onUnmounted(() => {
  document.removeEventListener('click', handleClickOutside)
  window.removeEventListener('resize', positionMenu)
  window.removeEventListener('scroll', positionMenu, true)
})
</script>

<template>
  <div ref="rootRef" class="split-button">
    <button
      class="btn btn-sm split-button-main"
      :class="`btn-${props.variant}`"
      :disabled="props.disabled"
      :aria-label="props.label"
      :title="mainTitle"
      @click.stop="emit('click')"
    >
      <slot>{{ props.label }}</slot>
    </button>
    <button
      v-if="props.options.length"
      class="btn btn-sm split-button-toggle"
      :class="[`btn-${props.variant}`, {open}]"
      type="button"
      aria-haspopup="menu"
      :aria-expanded="open ? 'true' : 'false'"
      :aria-label="props.menuLabel"
      @click="toggleMenu"
    >
      <svg
        width="10"
        height="10"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="3"
        aria-hidden="true"
      >
        <polyline points="6 9 12 15 18 9" />
      </svg>
    </button>

    <Teleport to="body">
      <div v-if="open" ref="menuRef" class="split-button-menu" :style="menuStyle" role="menu">
        <button
          v-for="option in props.options"
          :key="option.value"
          class="split-button-item"
          :disabled="option.disabled"
          role="menuitem"
          type="button"
          @click.stop="choose(option)"
        >
          {{ option.label }}
        </button>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.split-button {
  display: inline-flex;
  align-items: stretch;
}

.split-button-main {
  border-top-right-radius: 0;
  border-bottom-right-radius: 0;
}

.split-button-toggle {
  display: flex;
  align-items: center;
  justify-content: center;
  min-width: 22px;
  padding: 0 0.3rem;
  border-top-left-radius: 0;
  border-bottom-left-radius: 0;
  border-left: 1px solid rgba(0, 0, 0, 0.25);
}

.split-button-toggle.open svg {
  transform: rotate(180deg);
}

.split-button-toggle svg {
  transition: transform 0.15s ease;
}

.split-button-menu {
  position: fixed;
  z-index: 1000;
  /* Sized to its widest item rather than to a fixed 160px, which left a third of the
     menu empty to the right of a single short option. `min-width` is the floor that
     keeps a one-word option from becoming a sliver too small to aim at; `max-width`
     is the ceiling that keeps a long one inside the viewport, since `positionMenu`
     clamps the menu's position but not its size. */
  width: max-content;
  min-width: 7rem;
  max-width: min(18rem, calc(100vw - 16px));
  padding: 0.25rem;
  background: var(--surface-elevated);
  border: 1px solid var(--border);
  border-radius: 6px;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.4);
}

.split-button-item {
  display: block;
  width: 100%;
  padding: 0.45rem 0.6rem;
  background: none;
  border: none;
  border-radius: 4px;
  color: var(--text);
  font-size: 0.85rem;
  text-align: left;
  cursor: pointer;
}

.split-button-item:hover:not(:disabled) {
  background: var(--surface-hover, rgba(255, 255, 255, 0.08));
}

.split-button-item:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}
</style>
