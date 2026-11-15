export const DEFAULT_BASE_URL =
  import.meta.env.VITE_PAINTBOARD_BASE_URL ||
  import.meta.env.NEXT_PUBLIC_PAINTBOARD_BASE_URL ||
  'https://paintboard.luogu.me'

export const normalizeBaseUrl = (u?: string | null) => {
  if (!u) return ''
  let s = u.trim()
  while (s.endsWith('/')) s = s.slice(0, -1)
  return s
}
