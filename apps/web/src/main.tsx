import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { RouterProvider } from '@tanstack/react-router'
import { startTheme } from '@/lib/theme'
import { router } from '@/router'
import { startSessionSync } from '@/store/sessions'
import './index.css'

// Theme, Bewegung und Schriftgröße vor dem ersten Rendern (UX-005, WEB-016).
startTheme()
startSessionSync()
const queryClient = new QueryClient({ defaultOptions: { queries: { retry: 1, refetchOnWindowFocus: false } } })

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
    </QueryClientProvider>
  </StrictMode>,
)
