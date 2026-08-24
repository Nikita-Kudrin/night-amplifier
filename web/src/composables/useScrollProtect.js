import { ref } from 'vue'

const isScrolling = ref(false)
const isSliding = ref(false)
let isTouching = false
let startX = 0
let startY = 0

let listenersInitialized = false

export function useScrollProtect() {
  if (!listenersInitialized) {
    if (typeof window !== 'undefined') {
      window.addEventListener('touchstart', (e) => {
        if (e.touches.length !== 1) return
        isTouching = true
        isScrolling.value = false
        isSliding.value = false
        startX = e.touches[0].clientX
        startY = e.touches[0].clientY
      }, { passive: true, capture: true })

      window.addEventListener('touchmove', (e) => {
        if (!isTouching || isScrolling.value || isSliding.value || e.touches.length !== 1) return
        
        const deltaX = Math.abs(e.touches[0].clientX - startX)
        const deltaY = Math.abs(e.touches[0].clientY - startY)
        
        // Threshold of 5px to decide intent
        if (deltaY > deltaX && deltaY > 5) {
          isScrolling.value = true
        } else if (deltaX >= deltaY && deltaX > 5) {
          isSliding.value = true
        }
      }, { passive: true, capture: true })

      window.addEventListener('touchend', () => {
        isTouching = false
        setTimeout(() => {
          isScrolling.value = false
          isSliding.value = false
        }, 50)
      }, { passive: true, capture: true })
      
      window.addEventListener('touchcancel', () => {
        isTouching = false
        isScrolling.value = false
        isSliding.value = false
      }, { passive: true, capture: true })

      listenersInitialized = true
    }
  }

  const blockIfScrolling = (event, originalValue = undefined) => {
    // Block if definitively scrolling
    if (isScrolling.value) {
      restoreValue(event, originalValue)
      return true
    }
    
    // For range inputs (sliders), block premature events before intent is decided.
    // This prevents micro-jumps of the slider when the user first places their finger to scroll.
    if (isTouching && !isSliding.value && event && event.target && event.target.type === 'range') {
      restoreValue(event, originalValue)
      return true
    }
    
    return false
  }
  
  const restoreValue = (event, originalValue) => {
    if (originalValue !== undefined && event && event.target) {
      if (event.target.type === 'checkbox') {
         event.target.checked = originalValue
      } else {
         event.target.value = originalValue
      }
    }
  }

  return {
    isScrolling,
    blockIfScrolling
  }
}
