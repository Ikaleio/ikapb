import { createBrowserRouter } from 'react-router-dom'
import { RootLayout } from './root-layout'
import { HomeRoute } from './routes/home'
import { AdminRoute } from './routes/admin'

export const router = createBrowserRouter([
  {
    path: '/',
    element: <RootLayout />,
    children: [
      { index: true, element: <HomeRoute /> },
      { path: 'admin', element: <AdminRoute /> }
    ]
  }
])

