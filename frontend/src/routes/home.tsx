import { useEffect } from 'react'
import { PaintboardCanvas } from '@/components/paintboard-canvas'

export function HomeRoute() {
  useEffect(() => {
    const setViewportHeight = () => {
      document.documentElement.style.setProperty(
        '--app-viewport-height',
        `${window.innerHeight}px`
      )
    }

    setViewportHeight()
    window.addEventListener('resize', setViewportHeight)
    window.addEventListener('orientationchange', setViewportHeight)

    return () => {
      window.removeEventListener('resize', setViewportHeight)
      window.removeEventListener('orientationchange', setViewportHeight)
    }
  }, [])

  return (
    <main
      className="flex h-full w-full overflow-hidden bg-background"
      style={{ height: 'var(--app-viewport-height, 100vh)' }}
    >
      <PaintboardCanvas />
    </main>
  )
}

