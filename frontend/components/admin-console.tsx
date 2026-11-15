'use client'

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Link } from 'react-router-dom'
import { Button } from '@/components/ui/button'
import { Card } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { ThemeToggle } from '@/components/theme-toggle'
import { Toaster } from '@/components/ui/sonner'
import { DEFAULT_BASE_URL, normalizeBaseUrl } from '@/lib/config'
import { toast } from 'sonner'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { Badge } from '@/components/ui/badge'
import { Separator } from '@/components/ui/separator'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import {
  Server,
  Key,
  Palette,
  Shield,
  Activity,
  Info,
  Upload,
  Trash2,
  Ban,
  CheckCircle2,
  XCircle,
  RefreshCw,
  Plus,
  Eye,
  EyeOff,
  Edit,
} from 'lucide-react'

const ADMIN_KEY_STORAGE_KEY = 'paintboard_admin_key'

type TokenRecord = {
  token: string
  uid: number
  cooldown_ms: number
  issued_at_ms: number
  scopes?: string[]
  expire_at_ms?: number
  revoked?: boolean
  ip?: string | null
  last_paint_at_ms?: number | null
}

type TokensResponse = {
  items: TokenRecord[]
  total: number
}

type BanRecord = {
  uid: number
  reason: string | null
  created_at_ms: number
  expire_at_ms: number | null
}

type ConnectionStat = {
  ip: string
  connections: number
}

type FillResult = {
  written: number
  format: string
}

function unwrapApiData<T>(payload: any): T {
  if (payload && typeof payload === 'object' && 'code' in payload) {
    if ((payload as { code: number }).code !== 200) {
      const data = (payload as { data?: { error?: string; errorType?: string } }).data
      throw new Error(data?.error || data?.errorType || '请求失败')
    }
    return (payload as { data: T }).data
  }
  return payload as T
}

export function AdminConsole() {
  const [baseUrlInput, setBaseUrlInput] = useState(DEFAULT_BASE_URL)
  const [baseUrl, setBaseUrl] = useState(DEFAULT_BASE_URL)
  const [adminKey, setAdminKey] = useState('')
  const [showAdminKey, setShowAdminKey] = useState(false)
  const [activeSection, setActiveSection] = useState<
    'overview' | 'fills' | 'tokens' | 'bans' | 'connections'
  >('overview')

  const [fillsForm, setFillsForm] = useState({
    x: 0,
    y: 0,
    width: 100,
    height: 100,
    kind: 'color' as 'color' | 'blob' | 'url',
    color: '#000000',
    sourceValue: '',
    format: 'png',
    scaleMode: 'contain' as 'contain' | 'cover' | 'fill',
    scaleWidth: '',
    scaleHeight: '',
    grayscale: false
  })
  const [fillResult, setFillResult] = useState<FillResult | null>(null)
  const [isSubmittingFill, setIsSubmittingFill] = useState(false)
  const [isUploadingImage, setIsUploadingImage] = useState(false)
  const fileInputRef = useRef<HTMLInputElement | null>(null)

  const [tokens, setTokens] = useState<TokenRecord[]>([])
  const [allTokens, setAllTokens] = useState<TokenRecord[]>([])
  const [tokenTotal, setTokenTotal] = useState(0)
  const [isLoadingTokens, setIsLoadingTokens] = useState(false)
  const [tokenFilters, setTokenFilters] = useState({
    uid: '',
    revoked: 'all' as 'all' | 'true' | 'false',
    page: '0',
    pageSize: '20'
  })
  const [tokenForm, setTokenForm] = useState({
    uid: '',
    scopes: '',
    cooldownMs: '-1',
    expireAtMs: ''
  })
  const [editingToken, setEditingToken] = useState<TokenRecord | null>(null)
  const [editTokenForm, setEditTokenForm] = useState({
    uid: '',
    scopes: '',
    cooldownMs: '',
    expireAtMs: '',
    revoked: false
  })

  const [bans, setBans] = useState<BanRecord[]>([])
  const [isLoadingBans, setIsLoadingBans] = useState(false)
  const [banFilters, setBanFilters] = useState({ uid: '', activeOnly: true })
  const [banForm, setBanForm] = useState({ uid: '', reason: '', expireAtMs: '' })

  const [connections, setConnections] = useState<ConnectionStat[]>([])
  const [isLoadingConnections, setIsLoadingConnections] = useState(false)

  useEffect(() => {
    try {
      const storedBase =
        localStorage.getItem('paintboard_baseUrl') || DEFAULT_BASE_URL
      const storedAdminKey = localStorage.getItem(ADMIN_KEY_STORAGE_KEY) || ''
      setBaseUrl(normalizeBaseUrl(storedBase))
      setBaseUrlInput(storedBase)
      setAdminKey(storedAdminKey)
    } catch (error) {
      console.error('Failed to load persisted settings', error)
    }
  }, [])

  const saveConnectionSettings = useCallback(() => {
    const normalized = normalizeBaseUrl(baseUrlInput || DEFAULT_BASE_URL)
    setBaseUrl(normalized)
    try {
      localStorage.setItem('paintboard_baseUrl', baseUrlInput)
      localStorage.setItem(ADMIN_KEY_STORAGE_KEY, adminKey)
    } catch (error) {
      console.error('Unable to persist settings', error)
    }
    toast.success('连接配置已更新')
  }, [adminKey, baseUrlInput])

  const adminFetch = useCallback(
    async (path: string, init?: RequestInit) => {
      if (!baseUrl) throw new Error('请先填写后端地址')
      if (!adminKey) throw new Error('请填写管理密钥')

      const targetUrl = `${baseUrl}${path.startsWith('/') ? path : `/${path}`}`

      const response = await fetch(targetUrl, {
        method: init?.method || 'GET',
        headers: {
          ...(init?.headers as Record<string, string>),
          'Content-Type': 'application/json',
          'X-Admin-Key': adminKey
        },
        body: init?.body,
        cache: 'no-store'
      })

      const contentType = response.headers.get('content-type') || ''
      const payload = contentType.includes('application/json')
        ? await response.json()
        : await response.text()

      if (!response.ok) {
        const errorMessage =
          (payload as any)?.data?.error ||
          (payload as any)?.data?.errorType ||
          (typeof payload === 'string' ? payload : '') ||
          response.statusText
        throw new Error(errorMessage)
      }

      return unwrapApiData(payload)
    },
    [adminKey, baseUrl]
  )

  const submitFill = async () => {
    setIsSubmittingFill(true)
    try {
      const filters: Record<string, unknown> = {
        grayscale: fillsForm.grayscale
      }

      if (fillsForm.scaleWidth && fillsForm.scaleHeight) {
        filters.scale = {
          mode: fillsForm.scaleMode,
          width: Number(fillsForm.scaleWidth),
          height: Number(fillsForm.scaleHeight)
        }
      }

      const source: Record<string, unknown> = {
        kind: fillsForm.kind
      }

      if (fillsForm.kind === 'color') {
        source.value = fillsForm.color
      } else {
        if (!fillsForm.sourceValue) {
          throw new Error('请填写图片 Base64 或 URL')
        }
        source.value = fillsForm.sourceValue
        source.format = fillsForm.format
      }

      const body = {
        area: {
          x: Number(fillsForm.x),
          y: Number(fillsForm.y),
          width: Number(fillsForm.width),
          height: Number(fillsForm.height)
        },
        source,
        filters
      }

      const data = await adminFetch('/admin/fills', {
        method: 'POST',
        body: JSON.stringify(body)
      })
      setFillResult(data as FillResult)
      toast.success('填充请求已提交')
    } catch (error) {
      toast.error((error as Error).message)
    } finally {
      setIsSubmittingFill(false)
    }
  }

  const displayedSourceValue = useMemo(() => {
    if (fillsForm.kind !== 'blob') return fillsForm.sourceValue
    if (fillsForm.sourceValue.length <= 512) return fillsForm.sourceValue
    return `${fillsForm.sourceValue.slice(0, 256)}…（共 ${fillsForm.sourceValue.length} 字符）`
  }, [fillsForm.kind, fillsForm.sourceValue])

  const isSourceValueTruncated = useMemo(
    () => fillsForm.kind === 'blob' && fillsForm.sourceValue.length > 512,
    [fillsForm.kind, fillsForm.sourceValue.length]
  )

  const uploadImageAsBase64 = useCallback(
    async (file: File) => {
      setIsUploadingImage(true)
      try {
        const dataUrl = await new Promise<string>((resolve, reject) => {
          const reader = new FileReader()
          reader.onload = () => {
            if (typeof reader.result === 'string') {
              resolve(reader.result)
            } else {
              reject(new Error('无法读取文件'))
            }
          }
          reader.onerror = () => reject(new Error('读取文件失败'))
          reader.readAsDataURL(file)
        })

        const base64 = dataUrl.includes(',') ? dataUrl.split(',')[1] || '' : dataUrl
        if (!base64) throw new Error('无法解析 Base64 字符串')

        const inferFormat = (mime: string) => {
          if (mime.includes('png')) return 'png'
          if (mime.includes('jpeg')) return 'jpeg'
          if (mime.includes('jpg')) return 'jpg'
          if (mime.includes('webp')) return 'webp'
          return fillsForm.format
        }

        setFillsForm((prev) => ({
          ...prev,
          kind: 'blob',
          sourceValue: base64,
          format: inferFormat(file.type)
        }))
        toast.success('图片已转换为 Base64 并填入表单')
      } catch (error) {
        toast.error((error as Error).message)
      } finally {
        setIsUploadingImage(false)
        if (fileInputRef.current) {
          fileInputRef.current.value = ''
        }
      }
    },
    [fillsForm.format]
  )

  const fetchTokens = async () => {
    setIsLoadingTokens(true)
    try {
      // Fetch all tokens without filters for frontend filtering
      const data = (await adminFetch('/admin/tokens')) as TokensResponse
      setAllTokens(data.items)
      setTokens(data.items)
      setTokenTotal(data.total)
      toast.success('Token 列表已刷新')
    } catch (error) {
      toast.error((error as Error).message)
    } finally {
      setIsLoadingTokens(false)
    }
  }

  const createToken = async () => {
    if (!tokenForm.uid || !tokenForm.expireAtMs) {
      toast.error('UID 与过期时间必填')
      return
    }

    try {
      const body = {
        uid: Number(tokenForm.uid),
        scopes: tokenForm.scopes
          .split(',')
          .map((s) => s.trim())
          .filter(Boolean),
        cooldown_ms: Number(tokenForm.cooldownMs || -1),
        expire_at_ms: Number(tokenForm.expireAtMs)
      }

      await adminFetch('/admin/tokens', {
        method: 'POST',
        body: JSON.stringify(body)
      })
      toast.success('Token 已创建/替换')
      fetchTokens()
    } catch (error) {
      toast.error((error as Error).message)
    }
  }

  const toggleRevokeToken = async (token: TokenRecord) => {
    try {
      await adminFetch(`/admin/tokens/${token.token}`, {
        method: 'PATCH',
        body: JSON.stringify({ revoked: !token.revoked })
      })
      toast.success(token.revoked ? '已解除封禁' : '已撤销 Token')
      fetchTokens()
    } catch (error) {
      toast.error((error as Error).message)
    }
  }

  const deleteToken = async (token: string) => {
    try {
      await adminFetch(`/admin/tokens/${token}`, { method: 'DELETE' })
      toast.success('Token 已删除')
      fetchTokens()
    } catch (error) {
      toast.error((error as Error).message)
    }
  }

  const openEditDialog = (token: TokenRecord) => {
    setEditingToken(token)
    setEditTokenForm({
      uid: String(token.uid),
      scopes: token.scopes?.join(', ') ?? '',
      cooldownMs: String(token.cooldown_ms),
      expireAtMs: token.expire_at_ms != null ? String(token.expire_at_ms) : '',
      revoked: token.revoked ?? false
    })
  }

  const updateToken = async () => {
    if (!editingToken) return
    
    try {
      const body = {
        uid: Number(editTokenForm.uid),
        scopes: editTokenForm.scopes
          .split(',')
          .map((s) => s.trim())
          .filter(Boolean),
        cooldown_ms: Number(editTokenForm.cooldownMs),
        expire_at_ms: Number(editTokenForm.expireAtMs),
        revoked: editTokenForm.revoked
      }

      await adminFetch(`/admin/tokens/${editingToken.token}`, {
        method: 'PATCH',
        body: JSON.stringify(body)
      })
      toast.success('Token 已更新')
      setEditingToken(null)
      fetchTokens()
    } catch (error) {
      toast.error((error as Error).message)
    }
  }

  // Frontend filtering for tokens
  const filteredTokens = useMemo(() => {
    let filtered = [...allTokens]

    // Filter by UID
    if (tokenFilters.uid) {
      const uidFilter = tokenFilters.uid.toLowerCase()
      filtered = filtered.filter(t => String(t.uid).includes(uidFilter))
    }

    // Filter by revoked status
    if (tokenFilters.revoked !== 'all') {
      const isRevoked = tokenFilters.revoked === 'true'
      filtered = filtered.filter(
        (t) => (typeof t.revoked === 'boolean' ? t.revoked : false) === isRevoked
      )
    }

    return filtered
  }, [allTokens, tokenFilters.uid, tokenFilters.revoked])

  // Apply pagination on frontend
  const paginatedTokens = useMemo(() => {
    const page = Number(tokenFilters.page) || 0
    const pageSize = Number(tokenFilters.pageSize) || 20
    const start = page * pageSize
    const end = start + pageSize
    return filteredTokens.slice(start, end)
  }, [filteredTokens, tokenFilters.page, tokenFilters.pageSize])

  const fetchBans = async () => {
    setIsLoadingBans(true)
    try {
      const params = new URLSearchParams()
      if (banFilters.uid) params.set('uid', banFilters.uid)
      if (banFilters.activeOnly) params.set('active_only', 'true')

      const data = (await adminFetch(`/admin/bans?${params.toString()}`)) as {
        items: BanRecord[]
      }
      setBans(data.items)
      toast.success('封禁列表已刷新')
    } catch (error) {
      toast.error((error as Error).message)
    } finally {
      setIsLoadingBans(false)
    }
  }

  const saveBan = async () => {
    if (!banForm.uid) {
      toast.error('请填写 UID')
      return
    }

    try {
      const body = {
        uid: Number(banForm.uid),
        reason: banForm.reason || undefined,
        expire_at_ms: banForm.expireAtMs ? Number(banForm.expireAtMs) : null
      }

      await adminFetch('/admin/bans', {
        method: 'POST',
        body: JSON.stringify(body)
      })
      toast.success('封禁已创建/更新')
      fetchBans()
    } catch (error) {
      toast.error((error as Error).message)
    }
  }

  const removeBan = async (uid: number) => {
    try {
      await adminFetch(`/admin/bans/${uid}`, { method: 'DELETE' })
      toast.success('封禁记录已删除')
      fetchBans()
    } catch (error) {
      toast.error((error as Error).message)
    }
  }

  const fetchConnections = async () => {
    setIsLoadingConnections(true)
    try {
      const data = (await adminFetch('/admin/ws/connections')) as {
        items: ConnectionStat[]
      }
      setConnections(data.items)
      toast.success('连接统计已刷新')
    } catch (error) {
      toast.error((error as Error).message)
    } finally {
      setIsLoadingConnections(false)
    }
  }

  const activeSectionTitle = useMemo(() => {
    switch (activeSection) {
      case 'fills':
        return '画板填充'
      case 'tokens':
        return 'Token 管理'
      case 'bans':
        return '封禁管理'
      case 'connections':
        return '连接监控'
      default:
        return '总览'
    }
  }, [activeSection])

  return (
    <div className="min-h-screen bg-gradient-to-br from-background via-background to-muted/20">
      {/* Header */}
      <div className="border-b bg-card/80 backdrop-blur-sm sticky top-0 z-10 shadow-sm">
        <div className="mx-auto flex max-w-7xl items-center justify-between px-6 py-4">
          <div className="flex items-center gap-3">
            <div className="flex size-10 items-center justify-center rounded-lg bg-primary/10">
              <Server className="size-5 text-primary" />
            </div>
            <div>
              <p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
                Admin Console
              </p>
              <h1 className="text-xl font-bold">{activeSectionTitle}</h1>
            </div>
          </div>
          <div className="flex items-center gap-3">
            <ThemeToggle />
            <Button asChild variant="outline" size="sm">
              <Link to="/">返回画板</Link>
            </Button>
          </div>
        </div>
      </div>

      <div className="mx-auto max-w-7xl space-y-8 px-6 py-8">
        {/* Connection Settings Card */}
        <Card className="border-2 bg-card shadow-lg">
          <div className="border-b bg-muted/30 px-6 py-4">
            <div className="flex items-center gap-2">
              <Key className="size-5 text-primary" />
              <h2 className="text-lg font-semibold">连接配置</h2>
            </div>
            <p className="mt-1 text-sm text-muted-foreground">
              配置后端服务器地址和管理密钥以使用管理功能
            </p>
          </div>
          <div className="p-6">
            <div className="grid gap-6 md:grid-cols-2">
              <div className="space-y-2">
                <Label htmlFor="base-url" className="text-sm font-medium flex items-center gap-2">
                  <Server className="size-4" />
                  后端地址
                </Label>
                <Input
                  id="base-url"
                  value={baseUrlInput}
                  onChange={(e) => setBaseUrlInput(e.target.value)}
                  placeholder="https://example.com"
                  className="h-10"
                />
                <p className="text-xs text-muted-foreground">
                  当前: <span className="font-mono">{baseUrl || '未配置'}</span>
                </p>
              </div>
              <div className="space-y-2">
                <Label htmlFor="admin-key" className="text-sm font-medium flex items-center gap-2">
                  <Key className="size-4" />
                  管理密钥
                </Label>
                <div className="relative">
                  <Input
                    id="admin-key"
                    type={showAdminKey ? 'text' : 'password'}
                    value={adminKey}
                    onChange={(e) => setAdminKey(e.target.value)}
                    placeholder="输入管理密钥"
                    className="h-10 pr-10"
                  />
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon-sm"
                    className="absolute right-1 top-1/2 -translate-y-1/2"
                    onClick={() => setShowAdminKey(!showAdminKey)}
                  >
                    {showAdminKey ? (
                      <EyeOff className="size-4" />
                    ) : (
                      <Eye className="size-4" />
                    )}
                  </Button>
                </div>
                <p className="text-xs text-muted-foreground">
                  状态: {adminKey ? (
                    <Badge variant="success" className="ml-1">已填写</Badge>
                  ) : (
                    <Badge variant="destructive" className="ml-1">未填写</Badge>
                  )}
                </p>
              </div>
            </div>
            <Separator className="my-6" />
            <div className="flex items-center justify-between">
              <p className="text-sm text-muted-foreground">
                配置将保存在浏览器本地存储中
              </p>
              <div className="flex gap-3">
                <Button variant="outline" onClick={() => setBaseUrlInput(DEFAULT_BASE_URL)}>
                  重置地址
                </Button>
                <Button onClick={saveConnectionSettings}>
                  保存配置
                </Button>
              </div>
            </div>
          </div>
        </Card>

        {/* Tabs Navigation */}
        <Tabs value={activeSection} onValueChange={(v) => setActiveSection(v as typeof activeSection)}>
          <TabsList className="grid w-full grid-cols-5 h-auto p-1">
            <TabsTrigger value="overview" className="gap-2 py-2.5">
              <Info className="size-4" />
              <span className="hidden sm:inline">总览</span>
            </TabsTrigger>
            <TabsTrigger value="fills" className="gap-2 py-2.5">
              <Palette className="size-4" />
              <span className="hidden sm:inline">填充</span>
            </TabsTrigger>
            <TabsTrigger value="tokens" className="gap-2 py-2.5">
              <Key className="size-4" />
              <span className="hidden sm:inline">Token</span>
            </TabsTrigger>
            <TabsTrigger value="bans" className="gap-2 py-2.5">
              <Ban className="size-4" />
              <span className="hidden sm:inline">封禁</span>
            </TabsTrigger>
            <TabsTrigger value="connections" className="gap-2 py-2.5">
              <Activity className="size-4" />
              <span className="hidden sm:inline">连接</span>
            </TabsTrigger>
          </TabsList>

        {activeSection === 'overview' && (
          <TabsContent value="overview" className="space-y-6 mt-6">
            <div className="grid gap-6 md:grid-cols-2">
              <Card className="border-2 shadow-md overflow-hidden">
                <div className="border-b bg-primary/5 px-6 py-4">
                  <div className="flex items-center gap-2">
                    <Info className="size-5 text-primary" />
                    <h3 className="text-lg font-semibold">管理接口概览</h3>
                  </div>
                </div>
                <div className="p-6 space-y-4">
                  <p className="text-sm text-muted-foreground leading-relaxed">
                    控制台依照后端 API 规范实现，提供完整的管理功能接口。
                  </p>
                  <Separator />
                  <div className="space-y-3">
                    <div className="flex items-start gap-3">
                      <Palette className="size-5 text-primary mt-0.5 shrink-0" />
                      <div>
                        <p className="text-sm font-medium">画板填充</p>
                        <p className="text-xs text-muted-foreground">
                          支持纯色、图片以及灰度/缩放滤镜
                        </p>
                      </div>
                    </div>
                    <div className="flex items-start gap-3">
                      <Key className="size-5 text-primary mt-0.5 shrink-0" />
                      <div>
                        <p className="text-sm font-medium">Token 管理</p>
                        <p className="text-xs text-muted-foreground">
                          列表支持 UID 与撤销状态过滤
                        </p>
                      </div>
                    </div>
                    <div className="flex items-start gap-3">
                      <Ban className="size-5 text-primary mt-0.5 shrink-0" />
                      <div>
                        <p className="text-sm font-medium">封禁管理</p>
                        <p className="text-xs text-muted-foreground">
                          支持快速创建/删除封禁记录
                        </p>
                      </div>
                    </div>
                    <div className="flex items-start gap-3">
                      <Activity className="size-5 text-primary mt-0.5 shrink-0" />
                      <div>
                        <p className="text-sm font-medium">连接监控</p>
                        <p className="text-xs text-muted-foreground">
                          展示当前 WebSocket 连接统计
                        </p>
                      </div>
                    </div>
                  </div>
                </div>
              </Card>
              <Card className="border-2 shadow-md overflow-hidden">
                <div className="border-b bg-primary/5 px-6 py-4">
                  <div className="flex items-center gap-2">
                    <Server className="size-5 text-primary" />
                    <h3 className="text-lg font-semibold">当前配置</h3>
                  </div>
                </div>
                <div className="p-6 space-y-4">
                  <div className="space-y-4">
                    <div className="flex items-start justify-between gap-4">
                      <div className="space-y-1">
                        <p className="text-sm font-medium">后端地址</p>
                        <p className="text-xs font-mono text-muted-foreground break-all">
                          {baseUrl || '未配置'}
                        </p>
                      </div>
                      {baseUrl && (
                        <Badge variant="success">
                          <CheckCircle2 className="size-3 mr-1" />
                          已配置
                        </Badge>
                      )}
                    </div>
                    <Separator />
                    <div className="flex items-start justify-between gap-4">
                      <div className="space-y-1">
                        <p className="text-sm font-medium">管理密钥</p>
                        <p className="text-xs text-muted-foreground">
                          {adminKey ? '••••••••••••••••' : '未填写'}
                        </p>
                      </div>
                      {adminKey ? (
                        <Badge variant="success">
                          <CheckCircle2 className="size-3 mr-1" />
                          已填写
                        </Badge>
                      ) : (
                        <Badge variant="destructive">
                          <XCircle className="size-3 mr-1" />
                          未填写
                        </Badge>
                      )}
                    </div>
                    <Separator />
                    <div className="flex items-start justify-between gap-4">
                      <div className="space-y-1">
                        <p className="text-sm font-medium">本地持久化</p>
                        <p className="text-xs text-muted-foreground">
                          配置保存在浏览器中
                        </p>
                      </div>
                      <Badge variant="success">
                        <CheckCircle2 className="size-3 mr-1" />
                        已启用
                      </Badge>
                    </div>
                  </div>
                </div>
              </Card>
            </div>
          </TabsContent>
        )}

        {activeSection === 'fills' && (
          <TabsContent value="fills" className="space-y-6 mt-6">
            <Card className="border-2 shadow-md overflow-hidden">
              <div className="border-b bg-primary/5 px-6 py-4">
                <div className="flex items-center justify-between">
                  <div className="flex items-center gap-2">
                    <Palette className="size-5 text-primary" />
                    <div>
                      <h3 className="text-lg font-semibold">画板填充</h3>
                      <p className="text-sm text-muted-foreground">
                        支持纯色与图片填充（Base64 或 URL）
                      </p>
                    </div>
                  </div>
                  <Button onClick={submitFill} disabled={isSubmittingFill} size="lg">
                    {isSubmittingFill ? (
                      <>
                        <RefreshCw className="size-4 animate-spin" />
                        提交中...
                      </>
                    ) : (
                      <>
                        <CheckCircle2 className="size-4" />
                        执行填充
                      </>
                    )}
                  </Button>
                </div>
              </div>

              <div className="p-6 space-y-6">
                {/* Area Section */}
                <div className="space-y-4">
                  <h4 className="text-sm font-semibold text-muted-foreground uppercase tracking-wider">
                    填充区域
                  </h4>
                  <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
                    <div className="space-y-2">
                      <Label htmlFor="fill-x" className="text-sm font-medium">X 坐标</Label>
                      <Input
                        id="fill-x"
                        type="number"
                        value={fillsForm.x}
                        onChange={(e) => setFillsForm({ ...fillsForm, x: Number(e.target.value) })}
                        className="h-10"
                      />
                    </div>
                    <div className="space-y-2">
                      <Label htmlFor="fill-y" className="text-sm font-medium">Y 坐标</Label>
                      <Input
                        id="fill-y"
                        type="number"
                        value={fillsForm.y}
                        onChange={(e) => setFillsForm({ ...fillsForm, y: Number(e.target.value) })}
                        className="h-10"
                      />
                    </div>
                    <div className="space-y-2">
                      <Label htmlFor="fill-width" className="text-sm font-medium">宽度</Label>
                      <Input
                        id="fill-width"
                        type="number"
                        value={fillsForm.width}
                        onChange={(e) =>
                          setFillsForm({ ...fillsForm, width: Number(e.target.value) })
                        }
                        className="h-10"
                      />
                    </div>
                    <div className="space-y-2">
                      <Label htmlFor="fill-height" className="text-sm font-medium">高度</Label>
                      <Input
                        id="fill-height"
                        type="number"
                        value={fillsForm.height}
                        onChange={(e) =>
                          setFillsForm({ ...fillsForm, height: Number(e.target.value) })
                        }
                        className="h-10"
                      />
                    </div>
                  </div>
                </div>

                <Separator />

                {/* Source Section */}
                <div className="space-y-4">
                  <h4 className="text-sm font-semibold text-muted-foreground uppercase tracking-wider">
                    填充来源
                  </h4>
                  <div className="grid gap-4">
                    <div className="space-y-2">
                      <Label htmlFor="fill-kind" className="text-sm font-medium">来源类型</Label>
                      <Select
                        value={fillsForm.kind}
                        onValueChange={(value: typeof fillsForm.kind) =>
                          setFillsForm({ ...fillsForm, kind: value })
                        }
                      >
                        <SelectTrigger id="fill-kind" className="h-10">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="color">纯色</SelectItem>
                          <SelectItem value="blob">图片 Base64</SelectItem>
                          <SelectItem value="url">图片 URL</SelectItem>
                        </SelectContent>
                      </Select>
                    </div>

                    {fillsForm.kind === 'color' ? (
                      <div className="space-y-2">
                        <Label htmlFor="fill-color" className="text-sm font-medium">
                          颜色 (#RRGGBB)
                        </Label>
                        <Input
                          id="fill-color"
                          value={fillsForm.color}
                          onChange={(e) => setFillsForm({ ...fillsForm, color: e.target.value })}
                          placeholder="#000000"
                          className="h-10 font-mono"
                        />
                      </div>
                    ) : (
                      <div className="space-y-2">
                        <Label htmlFor="fill-source" className="text-sm font-medium">
                          {fillsForm.kind === 'url' ? '图片 URL (https)' : '图片 Base64'}
                        </Label>
                        <div className="flex flex-col gap-2 md:flex-row">
                          <Input
                            id="fill-source"
                            value={displayedSourceValue}
                            readOnly={isSourceValueTruncated}
                            onChange={(e) =>
                              setFillsForm({ ...fillsForm, sourceValue: e.target.value })
                            }
                            placeholder={
                              fillsForm.kind === 'url'
                                ? 'https://example.com/image.png'
                                : '不含 data: 前缀的 Base64 字符串'
                            }
                            title={
                              isSourceValueTruncated
                                ? '已折叠显示长字符串，重新上传或清空以编辑完整内容'
                                : undefined
                            }
                            className="h-10 flex-1"
                          />
                          {fillsForm.kind === 'blob' && (
                            <div className="flex items-center gap-2">
                              <input
                                ref={fileInputRef}
                                type="file"
                                accept="image/png,image/jpeg,image/webp,image/jpg"
                                className="hidden"
                                onChange={(e) => {
                                  const file = e.target.files?.[0]
                                  if (file) uploadImageAsBase64(file)
                                }}
                              />
                              <Button
                                type="button"
                                variant="outline"
                                onClick={() => fileInputRef.current?.click()}
                                disabled={isUploadingImage}
                                className="shrink-0"
                              >
                                {isUploadingImage ? (
                                  <>
                                    <RefreshCw className="size-4 animate-spin" />
                                    转换中...
                                  </>
                                ) : (
                                  <>
                                    <Upload className="size-4" />
                                    上传图片
                                  </>
                                )}
                              </Button>
                              {fillsForm.sourceValue && (
                                <Button
                                  type="button"
                                  variant="ghost"
                                  size="icon"
                                  onClick={() => setFillsForm({ ...fillsForm, sourceValue: '' })}
                                >
                                  <Trash2 className="size-4" />
                                </Button>
                              )}
                            </div>
                          )}
                        </div>
                        {isSourceValueTruncated && (
                          <p className="text-xs text-muted-foreground">
                            Base64 已折叠显示（长度 {fillsForm.sourceValue.length}）。提交时会使用完整内容。
                          </p>
                        )}
                      </div>
                    )}

                    {fillsForm.kind !== 'color' && (
                      <div className="space-y-2">
                        <Label htmlFor="fill-format" className="text-sm font-medium">图片格式</Label>
                        <Select
                          value={fillsForm.format}
                          onValueChange={(value) =>
                            setFillsForm({ ...fillsForm, format: value })
                          }
                        >
                          <SelectTrigger id="fill-format" className="h-10">
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            <SelectItem value="png">PNG</SelectItem>
                            <SelectItem value="jpg">JPG</SelectItem>
                            <SelectItem value="jpeg">JPEG</SelectItem>
                            <SelectItem value="webp">WEBP</SelectItem>
                          </SelectContent>
                        </Select>
                      </div>
                    )}
                  </div>
                </div>

                <Separator />

                {/* Filters Section */}
                <div className="space-y-4">
                  <h4 className="text-sm font-semibold text-muted-foreground uppercase tracking-wider">
                    图像滤镜
                  </h4>
                  <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
                    <div className="space-y-2">
                      <Label className="text-sm font-medium">灰度滤镜</Label>
                      <Button
                        variant={fillsForm.grayscale ? 'default' : 'outline'}
                        onClick={() => setFillsForm({ ...fillsForm, grayscale: !fillsForm.grayscale })}
                        className="w-full h-10"
                      >
                        {fillsForm.grayscale ? '已开启' : '关闭'}
                      </Button>
                    </div>
                    <div className="space-y-2">
                      <Label htmlFor="fill-scale-mode" className="text-sm font-medium">缩放模式</Label>
                      <Select
                        value={fillsForm.scaleMode}
                        onValueChange={(value: typeof fillsForm.scaleMode) =>
                          setFillsForm({ ...fillsForm, scaleMode: value })
                        }
                      >
                        <SelectTrigger id="fill-scale-mode" className="h-10">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="contain">Contain</SelectItem>
                          <SelectItem value="cover">Cover</SelectItem>
                          <SelectItem value="fill">Fill</SelectItem>
                        </SelectContent>
                      </Select>
                    </div>
                    <div className="space-y-2">
                      <Label htmlFor="fill-scale-width" className="text-sm font-medium">缩放宽度 (px)</Label>
                      <Input
                        id="fill-scale-width"
                        type="number"
                        value={fillsForm.scaleWidth}
                        onChange={(e) => setFillsForm({ ...fillsForm, scaleWidth: e.target.value })}
                        placeholder="可选"
                        className="h-10"
                      />
                    </div>
                    <div className="space-y-2">
                      <Label htmlFor="fill-scale-height" className="text-sm font-medium">缩放高度 (px)</Label>
                      <Input
                        id="fill-scale-height"
                        type="number"
                        value={fillsForm.scaleHeight}
                        onChange={(e) => setFillsForm({ ...fillsForm, scaleHeight: e.target.value })}
                        placeholder="可选"
                        className="h-10"
                      />
                    </div>
                  </div>
                </div>

                {fillResult && (
                  <>
                    <Separator />
                    <Card className="border-2 border-primary/20 bg-primary/5">
                      <div className="p-4 flex items-start gap-3">
                        <CheckCircle2 className="size-5 text-primary shrink-0 mt-0.5" />
                        <div className="space-y-1">
                          <p className="text-sm font-semibold">填充成功</p>
                          <p className="text-xs text-muted-foreground">
                            写入像素：{fillResult.written} | 格式：{fillResult.format}
                          </p>
                        </div>
                      </div>
                    </Card>
                  </>
                )}
              </div>
            </Card>
          </TabsContent>
        )}

        {activeSection === 'tokens' && (
          <div className="space-y-4">
            <Card className="p-5">
              <div className="flex flex-col gap-4 md:flex-row md:items-end md:justify-between">
                <div className="grid flex-1 gap-3 md:grid-cols-4">
                  <div className="space-y-1">
                    <Label htmlFor="token-uid">UID</Label>
                    <Input
                      id="token-uid"
                      value={tokenFilters.uid}
                      onChange={(e) => setTokenFilters({ ...tokenFilters, uid: e.target.value })}
                      placeholder="按 UID 过滤"
                    />
                  </div>
                  <div className="space-y-1">
                    <Label htmlFor="token-revoked">撤销状态</Label>
                    <select
                      id="token-revoked"
                      className="h-10 rounded-md border bg-background px-3"
                      value={tokenFilters.revoked}
                      onChange={(e) =>
                        setTokenFilters({ ...tokenFilters, revoked: e.target.value as typeof tokenFilters.revoked })
                      }
                    >
                      <option value="all">全部</option>
                      <option value="false">正常</option>
                      <option value="true">已撤销</option>
                    </select>
                  </div>
                  <div className="space-y-1">
                    <Label htmlFor="token-page">页码</Label>
                    <Input
                      id="token-page"
                      type="number"
                      value={tokenFilters.page}
                      onChange={(e) => setTokenFilters({ ...tokenFilters, page: e.target.value })}
                    />
                  </div>
                  <div className="space-y-1">
                    <Label htmlFor="token-page-size">每页条数</Label>
                    <Input
                      id="token-page-size"
                      type="number"
                      value={tokenFilters.pageSize}
                      onChange={(e) => setTokenFilters({ ...tokenFilters, pageSize: e.target.value })}
                    />
                  </div>
                </div>
                <Button onClick={fetchTokens} disabled={isLoadingTokens}>
                  {isLoadingTokens ? '加载中...' : '刷新列表'}
                </Button>
              </div>
            </Card>

            <Card className="p-5">
              <h3 className="text-lg font-semibold">创建/替换 Token</h3>
              <div className="mt-3 grid gap-3 md:grid-cols-4">
                <div className="space-y-1">
                  <Label htmlFor="create-token-uid">UID</Label>
                  <Input
                    id="create-token-uid"
                    value={tokenForm.uid}
                    onChange={(e) => setTokenForm({ ...tokenForm, uid: e.target.value })}
                  />
                </div>
                <div className="space-y-1">
                  <Label htmlFor="create-token-scopes">Scopes (逗号分隔)</Label>
                  <Input
                    id="create-token-scopes"
                    value={tokenForm.scopes}
                    onChange={(e) => setTokenForm({ ...tokenForm, scopes: e.target.value })}
                  />
                </div>
                <div className="space-y-1">
                  <Label htmlFor="create-token-cooldown">冷却(ms)</Label>
                  <Input
                    id="create-token-cooldown"
                    value={tokenForm.cooldownMs}
                    onChange={(e) => setTokenForm({ ...tokenForm, cooldownMs: e.target.value })}
                  />
                </div>
                <div className="space-y-1">
                  <Label htmlFor="create-token-expire">过期时间戳(ms)</Label>
                  <Input
                    id="create-token-expire"
                    value={tokenForm.expireAtMs}
                    onChange={(e) => setTokenForm({ ...tokenForm, expireAtMs: e.target.value })}
                    placeholder="必须大于 issued_at_ms"
                  />
                </div>
              </div>
              <div className="mt-3 flex justify-end">
                <Button onClick={createToken}>创建/替换</Button>
              </div>
            </Card>

            <Card className="p-5">
              <div className="flex items-center justify-between">
                <div>
                  <h3 className="text-lg font-semibold">Token 列表</h3>
                  <p className="text-sm text-muted-foreground">
                    显示 {paginatedTokens.length} / 过滤 {filteredTokens.length} / 总计 {tokenTotal} 条
                  </p>
                </div>
                <Button variant="outline" onClick={fetchTokens} disabled={isLoadingTokens}>
                  {isLoadingTokens ? '加载中...' : '重新加载'}
                </Button>
              </div>

              <div className="mt-4 overflow-x-auto">
                <table className="min-w-full text-sm">
                  <thead className="text-left text-muted-foreground">
                    <tr>
                      <th className="px-3 py-2">Token</th>
                      <th className="px-3 py-2">UID</th>
                      <th className="px-3 py-2">Scopes</th>
                      <th className="px-3 py-2">冷却</th>
                      <th className="px-3 py-2">过期</th>
                      <th className="px-3 py-2">状态</th>
                      <th className="px-3 py-2 text-right">操作</th>
                    </tr>
                  </thead>
                  <tbody>
                    {paginatedTokens.map((item) => (
                      <tr key={item.token} className="border-t hover:bg-muted/50 transition-colors">
                        <td className="px-3 py-2 font-mono text-xs break-all">{item.token}</td>
                        <td className="px-3 py-2">{item.uid}</td>
                        <td className="px-3 py-2">
                          {Array.isArray(item.scopes) && item.scopes.length > 0 ? item.scopes.join(', ') : '—'}
                        </td>
                        <td className="px-3 py-2">{item.cooldown_ms}</td>
                        <td className="px-3 py-2">{item.expire_at_ms ?? '—'}</td>
                        <td className="px-3 py-2">
                          {typeof item.revoked === 'boolean' ? (
                            <Badge variant={item.revoked ? 'destructive' : 'success'}>
                              {item.revoked ? '已撤销' : '有效'}
                            </Badge>
                          ) : (
                            <Badge variant="secondary">—</Badge>
                          )}
                        </td>
                        <td className="px-3 py-2 text-right">
                          <div className="flex justify-end gap-2">
                            <Button
                              size="sm"
                              variant="ghost"
                              onClick={() => openEditDialog(item)}
                              disabled={
                                !Array.isArray(item.scopes) &&
                                item.expire_at_ms == null &&
                                typeof item.revoked !== 'boolean'
                              }
                            >
                              <Edit className="size-4" />
                            </Button>
                            {typeof item.revoked === 'boolean' && (
                              <Button
                                size="sm"
                                variant="outline"
                                onClick={() => toggleRevokeToken(item)}
                              >
                                {item.revoked ? '解除' : '撤销'}
                              </Button>
                            )}
                            <Button
                              size="sm"
                              variant="destructive"
                              onClick={() => deleteToken(item.token)}
                            >
                              <Trash2 className="size-4" />
                            </Button>
                          </div>
                        </td>
                      </tr>
                    ))}
                    {tokens.length === 0 && (
                      <tr>
                        <td className="px-3 py-4 text-center text-muted-foreground" colSpan={7}>
                          暂无数据，点击上方“刷新列表”获取最新记录
                        </td>
                      </tr>
                    )}
                  </tbody>
                </table>
              </div>
            </Card>

            {/* Edit Token Dialog */}
            <Dialog open={!!editingToken} onOpenChange={(open) => !open && setEditingToken(null)}>
              <DialogContent className="sm:max-w-[500px]">
                <DialogHeader>
                  <DialogTitle>编辑 Token</DialogTitle>
                  <DialogDescription>
                    修改 Token 的配置。Token ID 无法更改。
                  </DialogDescription>
                </DialogHeader>
                <div className="grid gap-4 py-4">
                  <div className="space-y-2">
                    <Label htmlFor="edit-token-id" className="text-sm font-medium">Token ID</Label>
                    <Input
                      id="edit-token-id"
                      value={editingToken?.token || ''}
                      readOnly
                      disabled
                      className="font-mono text-xs"
                    />
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor="edit-token-uid" className="text-sm font-medium">UID</Label>
                    <Input
                      id="edit-token-uid"
                      type="number"
                      value={editTokenForm.uid}
                      onChange={(e) => setEditTokenForm({ ...editTokenForm, uid: e.target.value })}
                      className="h-10"
                    />
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor="edit-token-scopes" className="text-sm font-medium">
                      Scopes (逗号分隔)
                    </Label>
                    <Input
                      id="edit-token-scopes"
                      value={editTokenForm.scopes}
                      onChange={(e) => setEditTokenForm({ ...editTokenForm, scopes: e.target.value })}
                      placeholder="scope1, scope2"
                      className="h-10"
                    />
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor="edit-token-cooldown" className="text-sm font-medium">
                      冷却时间 (ms)
                    </Label>
                    <Input
                      id="edit-token-cooldown"
                      type="number"
                      value={editTokenForm.cooldownMs}
                      onChange={(e) => setEditTokenForm({ ...editTokenForm, cooldownMs: e.target.value })}
                      className="h-10"
                    />
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor="edit-token-expire" className="text-sm font-medium">
                      过期时间戳 (ms)
                    </Label>
                    <Input
                      id="edit-token-expire"
                      type="number"
                      value={editTokenForm.expireAtMs}
                      onChange={(e) => setEditTokenForm({ ...editTokenForm, expireAtMs: e.target.value })}
                      className="h-10"
                    />
                  </div>
                  <div className="flex items-center space-x-2">
                    <input
                      id="edit-token-revoked"
                      type="checkbox"
                      checked={editTokenForm.revoked}
                      onChange={(e) => setEditTokenForm({ ...editTokenForm, revoked: e.target.checked })}
                      className="size-4 rounded border-gray-300"
                    />
                    <Label htmlFor="edit-token-revoked" className="text-sm font-medium cursor-pointer">
                      已撤销
                    </Label>
                  </div>
                </div>
                <DialogFooter>
                  <Button variant="outline" onClick={() => setEditingToken(null)}>
                    取消
                  </Button>
                  <Button onClick={updateToken}>
                    保存更改
                  </Button>
                </DialogFooter>
              </DialogContent>
            </Dialog>
          </div>
        )}

        {activeSection === 'bans' && (
          <div className="space-y-4">
            <Card className="p-5">
              <div className="flex flex-col gap-4 md:flex-row md:items-end md:justify-between">
                <div className="grid flex-1 gap-3 md:grid-cols-3">
                  <div className="space-y-1">
                    <Label htmlFor="ban-filter-uid">UID</Label>
                    <Input
                      id="ban-filter-uid"
                      value={banFilters.uid}
                      onChange={(e) => setBanFilters({ ...banFilters, uid: e.target.value })}
                      placeholder="按 UID 过滤"
                    />
                  </div>
                  <div className="space-y-1">
                    <Label htmlFor="ban-filter-active">仅活跃</Label>
                    <div className="flex h-10 items-center gap-2 rounded-md border px-3">
                      <input
                        id="ban-filter-active"
                        type="checkbox"
                        checked={banFilters.activeOnly}
                        onChange={(e) => setBanFilters({ ...banFilters, activeOnly: e.target.checked })}
                      />
                      <span className="text-sm">只看未过期</span>
                    </div>
                  </div>
                </div>
                <Button onClick={fetchBans} disabled={isLoadingBans}>
                  {isLoadingBans ? '加载中...' : '刷新列表'}
                </Button>
              </div>
            </Card>

            <Card className="p-5">
              <h3 className="text-lg font-semibold">创建/更新封禁</h3>
              <div className="mt-3 grid gap-3 md:grid-cols-3">
                <div className="space-y-1">
                  <Label htmlFor="ban-uid">UID</Label>
                  <Input
                    id="ban-uid"
                    value={banForm.uid}
                    onChange={(e) => setBanForm({ ...banForm, uid: e.target.value })}
                  />
                </div>
                <div className="space-y-1">
                  <Label htmlFor="ban-reason">原因</Label>
                  <Input
                    id="ban-reason"
                    value={banForm.reason}
                    onChange={(e) => setBanForm({ ...banForm, reason: e.target.value })}
                    placeholder="可选"
                  />
                </div>
                <div className="space-y-1">
                  <Label htmlFor="ban-expire">过期时间戳(ms)</Label>
                  <Input
                    id="ban-expire"
                    value={banForm.expireAtMs}
                    onChange={(e) => setBanForm({ ...banForm, expireAtMs: e.target.value })}
                    placeholder="为空视为永久"
                  />
                </div>
              </div>
              <div className="mt-3 flex justify-end gap-2">
                <Button variant="outline" onClick={() => setBanForm({ uid: '', reason: '', expireAtMs: '' })}>
                  重置
                </Button>
                <Button onClick={saveBan}>保存封禁</Button>
              </div>
            </Card>

            <Card className="p-5">
              <div className="flex items-center justify-between">
                <div>
                  <h3 className="text-lg font-semibold">封禁列表</h3>
                  <p className="text-sm text-muted-foreground">按 UID 排序展示</p>
                </div>
                <Button variant="outline" onClick={fetchBans} disabled={isLoadingBans}>
                  {isLoadingBans ? '加载中...' : '重新加载'}
                </Button>
              </div>

              <div className="mt-4 overflow-x-auto">
                <table className="min-w-full text-sm">
                  <thead className="text-left text-muted-foreground">
                    <tr>
                      <th className="px-3 py-2">UID</th>
                      <th className="px-3 py-2">原因</th>
                      <th className="px-3 py-2">创建时间</th>
                      <th className="px-3 py-2">过期时间</th>
                      <th className="px-3 py-2 text-right">操作</th>
                    </tr>
                  </thead>
                  <tbody>
                    {bans.map((ban) => (
                      <tr key={`${ban.uid}-${ban.created_at_ms}`} className="border-t">
                        <td className="px-3 py-2">{ban.uid}</td>
                        <td className="px-3 py-2">{ban.reason || '—'}</td>
                        <td className="px-3 py-2">{ban.created_at_ms}</td>
                        <td className="px-3 py-2">{ban.expire_at_ms ?? '永久'}</td>
                        <td className="px-3 py-2 text-right">
                          <Button size="sm" variant="destructive" onClick={() => removeBan(ban.uid)}>
                            删除
                          </Button>
                        </td>
                      </tr>
                    ))}
                    {bans.length === 0 && (
                      <tr>
                        <td className="px-3 py-4 text-center text-muted-foreground" colSpan={5}>
                          暂无封禁记录，点击“刷新列表”以获取数据
                        </td>
                      </tr>
                    )}
                  </tbody>
                </table>
              </div>
            </Card>
          </div>
        )}

        {activeSection === 'connections' && (
          <Card className="p-5">
            <div className="flex items-center justify-between">
              <div>
                <h3 className="text-lg font-semibold">WebSocket 连接</h3>
                <p className="text-sm text-muted-foreground">来源 IP 及连接数</p>
              </div>
              <Button onClick={fetchConnections} disabled={isLoadingConnections}>
                {isLoadingConnections ? '加载中...' : '刷新数据'}
              </Button>
            </div>

            <div className="mt-4 overflow-x-auto">
              <table className="min-w-full text-sm">
                <thead className="text-left text-muted-foreground">
                  <tr>
                    <th className="px-3 py-2">IP</th>
                    <th className="px-3 py-2">连接数</th>
                  </tr>
                </thead>
                <tbody>
                  {connections.map((item) => (
                    <tr key={item.ip} className="border-t">
                      <td className="px-3 py-2">{item.ip}</td>
                      <td className="px-3 py-2">{item.connections}</td>
                    </tr>
                  ))}
                  {connections.length === 0 && (
                    <tr>
                      <td className="px-3 py-4 text-center text-muted-foreground" colSpan={2}>
                        暂无连接数据，点击上方“刷新数据”获取最新统计
                    </td>
                    </tr>
                  )}
                </tbody>
              </table>
            </div>
          </Card>
        )}
        </Tabs>
      </div>
      <Toaster />
    </div>
  )
}
