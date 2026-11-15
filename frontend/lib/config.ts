const viteBaseUrl = (import.meta as any).env?.VITE_PAINTBOARD_BASE_URL as
  | string
  | undefined

const legacyNextBaseUrl =
  typeof process !== 'undefined'
    ? ((process as any).env?.NEXT_PUBLIC_PAINTBOARD_BASE_URL as string | undefined)
    : undefined

export const DEFAULT_BASE_URL =
  viteBaseUrl || legacyNextBaseUrl || 'https://paintboard.luogu.me'

export const normalizeBaseUrl = (u?: string | null) => {
  if (!u) return ''
  let s = u.trim()
  while (s.endsWith('/')) s = s.slice(0, -1)
  return s
}
