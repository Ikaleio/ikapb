'use client'

import type React from 'react'

import { Suspense, lazy, useEffect, useRef, useState, useCallback } from 'react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Card } from '@/components/ui/card'
import {
	ZoomIn,
	ZoomOut,
	Maximize2,
	Palette,
	Check,
	PanelRightClose,
	PanelRightOpen,
	Flame
} from 'lucide-react'
import { cn } from '@/lib/utils'
import * as Slider from '@radix-ui/react-slider'
import { toast } from 'sonner'
import { Toaster } from '@/components/ui/sonner'
import { ThemeToggle } from '@/components/theme-toggle'
import { DEFAULT_BASE_URL, normalizeBaseUrl } from '@/lib/config'

const HeatmapSlider = lazy(() => import('@/components/heatmap-slider'))

const BOARD_WIDTH = 1000
const BOARD_HEIGHT = 600

export function PaintboardCanvas() {
	const canvasRef = useRef<HTMLCanvasElement>(null)
	const containerRef = useRef<HTMLDivElement>(null)
	const wsRef = useRef<WebSocket | null>(null)
	const boardDataRef = useRef<Uint8Array>(
		new Uint8Array(BOARD_WIDTH * BOARD_HEIGHT * 3)
	)
	const paintCallbacksRef = useRef<Map<number, (status: number) => void>>(
		new Map()
	)
	const hasLoadedBoardRef = useRef(false)
	// 热力图优化：复用 ImageData 和缓存暗化颜色
	const heatmapImageDataRef = useRef<ImageData | null>(null)
	const darkenedColorsRef = useRef<Uint8Array | null>(null)

	const [zoom, setZoom] = useState(1)
	const zoomRef = useRef(zoom)
	useEffect(() => {
		zoomRef.current = zoom
	}, [zoom])
	const [pan, setPan] = useState({ x: 0, y: 0 })
	const panRef = useRef(pan)
	useEffect(() => {
		panRef.current = pan
	}, [pan])
	const pendingPanRef = useRef<{ x: number; y: number } | null>(null)
	const panUpdateFrameRef = useRef<number | null>(null)
	const [isPanning, setIsPanning] = useState(false)
	const [panStart, setPanStart] = useState({ x: 0, y: 0 })
	const [hasMouseMoved, setHasMouseMoved] = useState(false)
	const [selectedColor, setSelectedColor] = useState('#ff0000')

	// 热力视图开关：默认关闭且不持久化
	const [isHeatmapEnabled, setIsHeatmapEnabled] = useState(false)
	const [heatmapFadeDuration, setHeatmapFadeDuration] = useState(7)
	const [heatmapBackgroundBrightness, setHeatmapBackgroundBrightness] =
		useState(30)
	const pixelTimestampsRef = useRef<Map<number, number>>(new Map())
	const heatmapAnimationRef = useRef<number | null>(null)
	const suppressClickRef = useRef(false)
	const suppressClickTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(
		null
	)
        const stopOverlayPropagation = useCallback((event: React.SyntheticEvent) => {
                event.stopPropagation()
                // 在部分浏览器上，触摸事件会同时触发指针/触摸原生事件，确保原生事件同样停止冒泡
                if ('stopPropagation' in event.nativeEvent) {
                        event.nativeEvent.stopPropagation()
                }
                if (typeof event.nativeEvent === 'object' && 'stopImmediatePropagation' in event.nativeEvent) {
                        event.nativeEvent.stopImmediatePropagation()
                }
        }, [])

	// Zoom helpers: clamp and zoom around an anchor point (in container coords)
	const MIN_ZOOM = 0.75
	const MAX_ZOOM = 10
	const clampZoom = useCallback(
		(z: number) => Math.min(Math.max(z, MIN_ZOOM), MAX_ZOOM),
		[]
	)

	const zoomAt = useCallback(
		(targetZoom: number, anchorX: number, anchorY: number) => {
			const currentZoom = zoomRef.current
			const newZoom = clampZoom(targetZoom)
			// If zoom unchanged, skip heavy work
			if (newZoom === currentZoom) return

			const originX = BOARD_WIDTH / 2
			const originY = BOARD_HEIGHT / 2
			const k = newZoom / currentZoom

			// Keep the anchor point stable in screen coords
			const currentPan = panRef.current
			const newPan = {
				x: anchorX - (anchorX - currentPan.x - originX) * k - originX,
				y: anchorY - (anchorY - currentPan.y - originY) * k - originY
			}

			panRef.current = newPan
			zoomRef.current = newZoom
			setPan(newPan)
			setZoom(newZoom)
		},
		[clampZoom]
	)

	const zoomAtViewportCenter = useCallback(
		(targetZoom: number) => {
			const el = containerRef.current
			if (!el) {
				const clamped = clampZoom(targetZoom)
				zoomRef.current = clamped
				setZoom(clamped)
				return
			}
			const rect = el.getBoundingClientRect()
			const anchorX = rect.width / 2
			const anchorY = rect.height / 2
			zoomAt(targetZoom, anchorX, anchorY)
		},
		[zoomAt, clampZoom]
	)

	// Touch gesture state
	const [touchStartPos, setTouchStartPos] = useState<{
		x: number
		y: number
	} | null>(null)
	const [lastTouchDistance, setLastTouchDistance] = useState<number | null>(
		null
	)
	const [isTouchDragging, setIsTouchDragging] = useState(false)
	// 设置面板始终显示（移除 showSettings 状态变量）
	const [uid, setUid] = useState<number | null>(null)
	const [accessKey, setAccessKey] = useState('')
	const [token, setToken] = useState('')
	const [wsStatus, setWsStatus] = useState<
		'disconnected' | 'connecting' | 'connected'
	>('disconnected')

	// 鼠标坐标跟踪
	const [hasPointerDevice, setHasPointerDevice] = useState(false)
	const [mousePixelCoords, setMousePixelCoords] = useState<{
		x: number
		y: number
	} | null>(null)
	const [isCoordVisible, setIsCoordVisible] = useState(false)
	const coordTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)

        // initialize baseUrl from localStorage to avoid initial flicker
        const [baseUrlInput, setBaseUrlInput] = useState(() => {
                try {
			return localStorage.getItem('paintboard_baseUrl') || DEFAULT_BASE_URL
		} catch (e) {
			return DEFAULT_BASE_URL
		}
	})
	const [baseUrl, setBaseUrl] = useState(() => {
		try {
			const raw = localStorage.getItem('paintboard_baseUrl') || DEFAULT_BASE_URL
			return normalizeBaseUrl(raw)
		} catch (e) {
			return normalizeBaseUrl(DEFAULT_BASE_URL)
		}
	})
	// versioning ref to prevent race conditions when baseUrl changes rapidly
	const baseChangeVersionRef = useRef(0)
	const [isApplyingBaseUrl, setIsApplyingBaseUrl] = useState(false)
	const retryCountRef = useRef(0)
	const maxRetries = 3

	// sidebar collapse state (auto-collapse on small screens)
	const [isSidebarCollapsed, setIsSidebarCollapsed] = useState(false)

	// 检测是否有鼠标设备
	useEffect(() => {
		const detectPointer = () => {
			// 检测是否有精细指针设备（鼠标）
			const hasFinePointer = window.matchMedia('(pointer: fine)').matches
			setHasPointerDevice(hasFinePointer)
		}

		detectPointer()

		// 监听指针设备变化
		const mediaQuery = window.matchMedia('(pointer: fine)')
		const handleChange = () => detectPointer()
		mediaQuery.addEventListener('change', handleChange)

		return () => {
			mediaQuery.removeEventListener('change', handleChange)
		}
	}, [])

	// 标记是否已完成初始化
	const [isInitialized, setIsInitialized] = useState(false)

	useEffect(() => {
		const savedToken = localStorage.getItem('paintboard_token')
		const savedUid = localStorage.getItem('paintboard_uid')
		const savedHeatmapDuration = localStorage.getItem(
			'paintboard_heatmapFadeDuration'
		)
		const savedHeatmapBrightness = localStorage.getItem(
			'paintboard_heatmapBackgroundBrightness'
		)
		const savedSidebarCollapsed = localStorage.getItem(
			'paintboard_sidebarCollapsed'
		)

		// baseUrl is initialized from localStorage in useState lazy initializer to
		// avoid initial render using DEFAULT_BASE_URL then switching — that caused
		// flicker. Here we only restore token/uid.
		if (savedToken) setToken(savedToken)
		if (savedUid) {
			const parsedUid = Number.parseInt(savedUid, 10)
			if (!Number.isNaN(parsedUid)) {
				setUid(parsedUid)
			}
		}
		if (savedHeatmapDuration) {
			const duration = Number.parseInt(savedHeatmapDuration, 10)
			if (!Number.isNaN(duration)) {
				setHeatmapFadeDuration(duration)
			}
		}
		if (savedHeatmapBrightness) {
			const brightness = Number.parseInt(savedHeatmapBrightness, 10)
			if (!Number.isNaN(brightness)) {
				setHeatmapBackgroundBrightness(brightness)
			}
		}
		if (savedSidebarCollapsed !== null) {
			setIsSidebarCollapsed(savedSidebarCollapsed === 'true')
		} else {
			// 如果没有保存的状态，在小屏幕上自动折叠
			setIsSidebarCollapsed(window.innerWidth < 1024)
		}

		// 标记初始化完成
		setIsInitialized(true)
	}, [])

	useEffect(() => {
		if (baseUrl) {
			localStorage.setItem('paintboard_baseUrl', baseUrl)
		}
	}, [baseUrl])

	useEffect(() => {
		if (token) {
			localStorage.setItem('paintboard_token', token)
		}
	}, [token])

	useEffect(() => {
		if (uid !== null) {
			localStorage.setItem('paintboard_uid', uid.toString())
		}
	}, [uid])

	// 不持久化热力开关

	useEffect(() => {
		localStorage.setItem(
			'paintboard_heatmapFadeDuration',
			String(heatmapFadeDuration)
		)
	}, [heatmapFadeDuration])

	useEffect(() => {
		localStorage.setItem(
			'paintboard_heatmapBackgroundBrightness',
			String(heatmapBackgroundBrightness)
		)
	}, [heatmapBackgroundBrightness])

	useEffect(() => {
		localStorage.setItem(
			'paintboard_sidebarCollapsed',
			String(isSidebarCollapsed)
		)
	}, [isSidebarCollapsed])

	const handleApplyBaseUrl = useCallback(() => {
		if (baseUrlInput !== baseUrl && !isApplyingBaseUrl) {
			// bump version so any in-flight requests for previous baseUrl are ignored
			baseChangeVersionRef.current += 1
			hasLoadedBoardRef.current = false
			setIsApplyingBaseUrl(true)

			// immediately clear board to gray to indicate transition
			clearBoardToGray()

			// close existing ws to speed up switching
			if (wsRef.current) {
				try {
					wsRef.current.close()
				} catch (e) {
					/* ignore */
				}
				wsRef.current = null
			}

			setBaseUrl(normalizeBaseUrl(baseUrlInput))
			toast('Base URL 已更新', {
				description: '正在重新连接...'
			})

			// ensure we unset applying flag once this version's initial load completes
			// getBoard/connectWebSocket will check version and clear the flag when ready
			// (or after failure paths)
		}
	}, [baseUrlInput, baseUrl, isApplyingBaseUrl])

	const getBoard = useCallback(
		async (options?: { preserveView?: boolean }) => {
			const preserveView = options?.preserveView ?? false
			const viewToRestore = preserveView
				? {
						pan: { ...panRef.current },
						zoom: zoomRef.current
				  }
				: null
			const myVersion = baseChangeVersionRef.current
			if (!baseUrl) return

			try {
				console.log('[v0] Fetching board data from:', baseUrl)
				const response = await fetch(`${baseUrl}/api/paintboard/getboard`)

				if (!response.ok) {
					throw new Error(`HTTP error! status: ${response.status}`)
				}

				const arrayBuffer = await response.arrayBuffer()
				const data = new Uint8Array(arrayBuffer)

				if (data.length !== BOARD_WIDTH * BOARD_HEIGHT * 3) {
					throw new Error(
						`Invalid board data size: expected ${
							BOARD_WIDTH * BOARD_HEIGHT * 3
						}, got ${data.length}`
					)
				}

				boardDataRef.current = data
				renderBoard()

				// ignore if another baseUrl change happened while we were fetching
				if (myVersion !== baseChangeVersionRef.current) {
					console.log('[v0] Ignoring board data from stale baseUrl fetch')
					return
				}

				console.log('[v0] Board data loaded successfully')

				if (preserveView && viewToRestore) {
					const { pan: storedPan, zoom: storedZoom } = viewToRestore

					if (zoomRef.current !== storedZoom) {
						zoomRef.current = storedZoom
						setZoom(prev => (prev === storedZoom ? prev : storedZoom))
					}

					const currentPan = panRef.current
					if (currentPan.x !== storedPan.x || currentPan.y !== storedPan.y) {
						const panToApply = {
							x: storedPan.x,
							y: storedPan.y
						}
						panRef.current = panToApply
						setPan(prev =>
							prev.x === panToApply.x && prev.y === panToApply.y
								? prev
								: panToApply
						)
					}
				}

				toast.success('绘版加载成功', {
					description: '已获取最新绘版数据'
				})
				hasLoadedBoardRef.current = true
				setIsApplyingBaseUrl(false)
			} catch (error) {
				console.error('[v0] Failed to fetch board:', error)
				toast.error('加载失败', {
					description: `无法获取绘版数据: ${
						error instanceof Error ? error.message : '未知错误'
					}`
				})
				setIsApplyingBaseUrl(false)
			}
		},
		[baseUrl]
	)

	useEffect(() => {
		if (baseUrl) {
			getBoard()
		}
	}, [baseUrl, getBoard])

	useEffect(() => {
		if (!baseUrl) {
			setWsStatus('disconnected')
			return
		}

		retryCountRef.current = 0

		const connectWebSocket = () => {
			const myVersion = baseChangeVersionRef.current
			if (retryCountRef.current >= maxRetries) {
				console.log('[v0] Max WebSocket retry attempts reached')
				setWsStatus('disconnected')
				toast.error('连接失败', {
					description:
						'WebSocket 连接失败，已达到最大重试次数。请检查 Base URL 是否正确。'
				})
				return
			}

			try {
				setWsStatus('connecting')
				const wsUrl = baseUrl.replace(/^http/, 'ws') + '/api/paintboard/ws'
				console.log('[v0] Connecting to WebSocket:', wsUrl)
				const ws = new WebSocket(wsUrl)
				ws.binaryType = 'arraybuffer'

				ws.onopen = () => {
					// ignore if baseUrl changed while connecting
					if (myVersion !== baseChangeVersionRef.current) {
						console.log(
							'[v0] Ignoring websocket open for stale baseUrl version'
						)
						try {
							ws.close()
						} catch (e) {
							/* ignore */
						}
						return
					}
					console.log('[v0] WebSocket connected')
					setWsStatus('connected')
					retryCountRef.current = 0
					setIsApplyingBaseUrl(false)
					toast.success('已连接', {
						description: 'WebSocket 连接成功'
					})

					if (hasLoadedBoardRef.current) {
						// Re-sync board data after reconnect without snapping the viewport
						void getBoard({ preserveView: true })
					}
				}

				ws.onmessage = event => {
					// ignore messages if baseUrl changed
					if (myVersion !== baseChangeVersionRef.current) return
					const buffer = event.data as ArrayBuffer
					const dataView = new DataView(buffer)
					let offset = 0

					while (offset < buffer.byteLength) {
						const type = dataView.getUint8(offset)
						offset += 1

						switch (type) {
							case 0xfa: {
								const x = dataView.getUint16(offset, true)
								const y = dataView.getUint16(offset + 2, true)
								const r = dataView.getUint8(offset + 4)
								const g = dataView.getUint8(offset + 5)
								const b = dataView.getUint8(offset + 6)
								offset += 7

								const idx = (y * BOARD_WIDTH + x) * 3
								boardDataRef.current[idx] = r
								boardDataRef.current[idx + 1] = g
								boardDataRef.current[idx + 2] = b

								// Heatmap timestamp based solely on server updates
								pixelTimestampsRef.current.set(y * BOARD_WIDTH + x, Date.now())

								updatePixel(x, y, r, g, b)
								break
							}
							case 0xfc: {
								ws.send(new Uint8Array([0xfb]))
								break
							}
							case 0xff: {
								const id = dataView.getUint32(offset, true)
								const code = dataView.getUint8(offset + 4)
								offset += 5

								const callback = paintCallbacksRef.current.get(id)
								if (callback) {
									callback(code)
									paintCallbacksRef.current.delete(id)
								}
								break
							}
							default:
								console.log(`[v0] Unknown message type: ${type}`)
						}
					}
				}

				ws.onerror = error => {
					if (myVersion !== baseChangeVersionRef.current) return
					console.error('[v0] WebSocket error:', error)
					setWsStatus('disconnected')
					setIsApplyingBaseUrl(false)
					toast.error('WebSocket 错误', {
						description: '连接失败，请检查 Base URL 是否正确'
					})
				}

				ws.onclose = event => {
					if (myVersion !== baseChangeVersionRef.current) return
					console.log(
						`[v0] WebSocket closed (${event.code}: ${
							event.reason || 'Unknown'
						})`
					)
					setWsStatus('disconnected')
					setIsApplyingBaseUrl(false)

					retryCountRef.current += 1
					if (retryCountRef.current < maxRetries) {
						const retryDelay = Math.min(
							1000 * Math.pow(2, retryCountRef.current),
							10000
						)
						console.log(
							`[v0] Retrying WebSocket connection in ${retryDelay}ms (attempt ${
								retryCountRef.current + 1
							}/${maxRetries})`
						)
						setTimeout(connectWebSocket, retryDelay)
					}
				}

				wsRef.current = ws
			} catch (error) {
				if (myVersion !== baseChangeVersionRef.current) return
				console.error('[v0] Failed to connect WebSocket:', error)
				setWsStatus('disconnected')
				setIsApplyingBaseUrl(false)
				toast.error('连接失败', {
					description: `无法建立 WebSocket 连接: ${
						error instanceof Error ? error.message : '未知错误'
					}`
				})
			}
		}

		connectWebSocket()

		return () => {
			if (wsRef.current) {
				wsRef.current.close()
			}
		}
	}, [baseUrl, getBoard])

	const renderBoard = useCallback(() => {
		const canvas = canvasRef.current
		if (!canvas) return

		const ctx = canvas.getContext('2d')
		if (!ctx) return

		const imageData = ctx.createImageData(BOARD_WIDTH, BOARD_HEIGHT)
		const data = imageData.data

		for (let i = 0; i < BOARD_WIDTH * BOARD_HEIGHT; i++) {
			const srcIdx = i * 3
			const dstIdx = i * 4
			data[dstIdx] = boardDataRef.current[srcIdx]
			data[dstIdx + 1] = boardDataRef.current[srcIdx + 1]
			data[dstIdx + 2] = boardDataRef.current[srcIdx + 2]
			data[dstIdx + 3] = 255
		}

		ctx.putImageData(imageData, 0, 0)
	}, [])

	// fill boardDataRef and canvas with #AAAAAA gray
	const clearBoardToGray = useCallback(() => {
		const grayR = 0xaa
		const grayG = 0xaa
		const grayB = 0xaa
		for (let i = 0; i < BOARD_WIDTH * BOARD_HEIGHT; i++) {
			const idx = i * 3
			boardDataRef.current[idx] = grayR
			boardDataRef.current[idx + 1] = grayG
			boardDataRef.current[idx + 2] = grayB
		}

		const canvas = canvasRef.current
		if (!canvas) return
		const ctx = canvas.getContext('2d')
		if (!ctx) return
		const imageData = ctx.createImageData(BOARD_WIDTH, BOARD_HEIGHT)
		const data = imageData.data

		for (let i = 0; i < BOARD_WIDTH * BOARD_HEIGHT; i++) {
			const dst = i * 4
			data[dst] = grayR
			data[dst + 1] = grayG
			data[dst + 2] = grayB
			data[dst + 3] = 255
		}

		ctx.putImageData(imageData, 0, 0)
	}, [])

	const updatePixel = useCallback(
		(x: number, y: number, r: number, g: number, b: number) => {
			const canvas = canvasRef.current
			if (!canvas) return

			const ctx = canvas.getContext('2d')
			if (!ctx) return

			ctx.fillStyle = `rgb(${r}, ${g}, ${b})`
			ctx.fillRect(x, y, 1, 1)
		},
		[]
	)

	const handleCanvasClick = useCallback(
		(e: React.MouseEvent<HTMLCanvasElement>) => {
			if (suppressClickRef.current) {
				return
			}
			// If mouse moved during pan, don't paint
			if (hasMouseMoved) {
				return
			}

			if (!token) {
				toast.error('未设置 Token', {
					description: '请先获取或在设置中输入 Token'
				})
				return
			}

			if (!uid) {
				toast.error('未设置 UID', {
					description: '已输入 Token，但未填写 UID，请在设置面板中输入 UID'
				})
				return
			}

			const canvas = canvasRef.current
			if (!canvas) return

			const rect = canvas.getBoundingClientRect()
			// 注意：当 transformOrigin 为画布中心，且 transform: translate(...) scale(...) 时，
			// 以变换后外接矩形的左上角为参考（rect.left/top），点击位置与画布本地坐标的关系简化为：
			// click = zoom * localCoord，因此 localCoord = click / zoom。
			// 不需要额外减去 pan 或中心偏移。
			const clickX = e.clientX - rect.left
			const clickY = e.clientY - rect.top
			const x = Math.floor(clickX / zoom)
			const y = Math.floor(clickY / zoom)

			if (x < 0 || x >= BOARD_WIDTH || y < 0 || y >= BOARD_HEIGHT) return

			const color = selectedColor
			const r = Number.parseInt(color.slice(1, 3), 16)
			const g = Number.parseInt(color.slice(3, 5), 16)
			const b = Number.parseInt(color.slice(5, 7), 16)

			if (!token || uid === null) {
				console.error('[v0] Cannot paint: missing token or UID')
				return
			}

			paintPixel(uid, token, r, g, b, x, y)
		},
		[token, uid, zoom, pan, selectedColor, hasMouseMoved]
	)

	const paintPixel = useCallback(
		(
			uid: number,
			token: string,
			r: number,
			g: number,
			b: number,
			x: number,
			y: number
		) => {
			const ws = wsRef.current
			if (!ws || ws.readyState !== WebSocket.OPEN) {
				toast.error('WebSocket 未连接', {
					description: '请等待连接建立'
				})
				return
			}

			const id = Math.floor(Math.random() * 4294967296)

			console.log('[v0] Token string:', token)
			console.log('[v0] Token length:', token.length)

			const tokenBytes = new Uint8Array(16)
			const tokenHex = token.replace(/-/g, '')
			console.log('[v0] Token hex (no dashes):', tokenHex)
			console.log('[v0] Token hex length:', tokenHex.length)

			tokenHex.match(/.{2}/g)?.forEach((byte, i) => {
				tokenBytes[i] = Number.parseInt(byte, 16)
			})

			console.log(
				'[v0] Token bytes:',
				Array.from(tokenBytes)
					.map(b => b.toString(16).padStart(2, '0'))
					.join('')
			)

			const paintData = new Uint8Array(31)
			paintData[0] = 0xfe
			paintData[1] = x & 0xff
			paintData[2] = (x >> 8) & 0xff
			paintData[3] = y & 0xff
			paintData[4] = (y >> 8) & 0xff
			paintData[5] = r
			paintData[6] = g
			paintData[7] = b
			paintData[8] = uid & 0xff
			paintData[9] = (uid >> 8) & 0xff
			paintData[10] = (uid >> 16) & 0xff
			paintData.set(tokenBytes, 11)
			paintData[27] = id & 0xff
			paintData[28] = (id >> 8) & 0xff
			paintData[29] = (id >> 16) & 0xff
			paintData[30] = (id >> 24) & 0xff

			console.log(
				'[v0] Paint data:',
				Array.from(paintData)
					.map(b => b.toString(16).padStart(2, '0'))
					.join(' ')
			)
			console.log('[v0] Paint data breakdown:')
			console.log('[v0]   Opcode:', paintData[0].toString(16))
			console.log(
				'[v0]   X:',
				x,
				'->',
				paintData[1].toString(16),
				paintData[2].toString(16)
			)
			console.log(
				'[v0]   Y:',
				y,
				'->',
				paintData[3].toString(16),
				paintData[4].toString(16)
			)
			console.log('[v0]   RGB:', r, g, b)
			console.log(
				'[v0]   UID:',
				uid,
				'->',
				paintData[8].toString(16),
				paintData[9].toString(16),
				paintData[10].toString(16)
			)
			console.log(
				'[v0]   Token bytes:',
				Array.from(paintData.slice(11, 27))
					.map(b => b.toString(16).padStart(2, '0'))
					.join('')
			)
			console.log(
				'[v0]   ID:',
				id,
				'->',
				paintData[27].toString(16),
				paintData[28].toString(16),
				paintData[29].toString(16),
				paintData[30].toString(16)
			)

			// Optimistic update for visual feedback (no heatmap timestamp here)
			const baseIdx = (y * BOARD_WIDTH + x) * 3
			const prevR = boardDataRef.current[baseIdx]
			const prevG = boardDataRef.current[baseIdx + 1]
			const prevB = boardDataRef.current[baseIdx + 2]
			boardDataRef.current[baseIdx] = r
			boardDataRef.current[baseIdx + 1] = g
			boardDataRef.current[baseIdx + 2] = b
			// draw instantly (will be overwritten by animation frame if enabled)
			updatePixel(x, y, r, g, b)

			paintCallbacksRef.current.set(id, status => {
				console.log(
					'[v0] Paint result - ID:',
					id,
					'Status:',
					status.toString(16)
				)

				const messages: Record<number, string> = {
					239: '绘画成功',
					238: '正在冷却',
					237: 'Token 无效',
					236: '请求格式错误',
					235: '无权限',
					234: '服务器错误'
				}

				// 仅在非成功时回滚并显示提示，成功不需要弹出提示
				if (status !== 0xef) {
					// rollback board data
					boardDataRef.current[baseIdx] = prevR
					boardDataRef.current[baseIdx + 1] = prevG
					boardDataRef.current[baseIdx + 2] = prevB
					// redraw reverted pixel
					updatePixel(x, y, prevR, prevG, prevB)
					toast.error('绘画失败', {
						description: messages[status] || `未知状态码: ${status}`
					})
				}
			})

			ws.send(paintData)
			console.log('[v0] Paint data sent')
		},
		[updatePixel]
	)

	const handleGetToken = useCallback(async () => {
		if (uid === null || !accessKey) {
			toast.error('参数缺失', {
				description: '请输入 UID 和 AccessKey'
			})
			return
		}

		if (!baseUrl) {
			toast.error('未设置 Base URL', {
				description: '请先设置 Base URL'
			})
			return
		}

		try {
			const response = await fetch(`${baseUrl}/api/auth/gettoken`, {
				method: 'POST',
				headers: {
					'Content-Type': 'application/json'
				},
				body: JSON.stringify({
					uid,
					access_key: accessKey
				})
			})

			if (!response.ok) {
				throw new Error(`HTTP error! status: ${response.status}`)
			}

			const data = await response.json()

			if (response.status === 200 && data.data?.token) {
				setToken(data.data.token)
				toast.success('获取成功', {
					description: 'Token 已设置'
				})
			} else if (response.status === 403) {
				toast.error('获取失败', {
					description: data.data?.errorType || '未知错误'
				})
			} else {
				toast.error('响应异常', {
					description: `未预期的响应码: ${data.code || '未知'}`
				})
			}
		} catch (error) {
			console.error('[v0] Failed to get token:', error)
			toast.error('请求失败', {
				description: `无法获取 Token: ${
					error instanceof Error ? error.message : '未知错误'
				}`
			})
		}
	}, [uid, accessKey, baseUrl])

	const handleZoomIn = () => zoomAtViewportCenter(zoom * 1.5)
	const handleZoomOut = () => zoomAtViewportCenter(zoom / 1.5)

	// Compute pan so that the canvas is centered within the container viewport
	const computeCenterPan = useCallback((targetZoom: number) => {
		const el = containerRef.current
		if (!el) return { x: 0, y: 0 }
		const rect = el.getBoundingClientRect()
		// viewport center in container coords
		const vx = rect.width / 2
		const vy = rect.height / 2
		// canvas center in canvas coords (origin is top-left of canvas)
		const cx = BOARD_WIDTH / 2
		const cy = BOARD_HEIGHT / 2
		// We want: screen_anchor = pan + origin + (canvasPoint - origin) * scale
		// Solve for pan so that canvas center maps to viewport center.
		// pan = viewportCenter - origin - (canvasCenter - origin) * scale
		const originX = BOARD_WIDTH / 2
		const originY = BOARD_HEIGHT / 2
		const k = targetZoom
		const panX = vx - originX - (cx - originX) * k
		const panY = vy - originY - (cy - originY) * k
		return { x: panX, y: panY }
	}, [])

	const handleResetView = () => {
		// center at zoom = 1
		const targetZoom = 1
		const newPan = computeCenterPan(targetZoom)
		zoomRef.current = targetZoom
		panRef.current = newPan
		setZoom(targetZoom)
		setPan(newPan)
	}

	// 初始化完成后，等待容器渲染完成再居中画布
	useEffect(() => {
		if (!isInitialized) return

		// 使用 setTimeout 确保 DOM 已更新（侧边栏动画完成）
		const timeoutId = setTimeout(() => {
			if (containerRef.current) {
				const targetZoom = 1
				const newPan = computeCenterPan(targetZoom)
				zoomRef.current = targetZoom
				panRef.current = newPan
				setZoom(targetZoom)
				setPan(newPan)
			}
		}, 0)

		return () => clearTimeout(timeoutId)
	}, [isInitialized, computeCenterPan])

	const handleMouseDown = (e: React.MouseEvent) => {
		if (e.button === 1 || e.shiftKey) {
			setIsPanning(true)
			setHasMouseMoved(false)
			const currentPan = panRef.current
			setPanStart({
				x: e.clientX - currentPan.x,
				y: e.clientY - currentPan.y
			})
			e.preventDefault()
		}
	}

	const schedulePanUpdate = useCallback((nextPan: { x: number; y: number }) => {
		panRef.current = nextPan
		pendingPanRef.current = nextPan
		if (panUpdateFrameRef.current !== null) return

		panUpdateFrameRef.current = requestAnimationFrame(() => {
			if (pendingPanRef.current) {
				setPan(pendingPanRef.current)
				pendingPanRef.current = null
			}
			panUpdateFrameRef.current = null
		})
	}, [])

	useEffect(() => {
		return () => {
			if (panUpdateFrameRef.current !== null) {
				cancelAnimationFrame(panUpdateFrameRef.current)
				panUpdateFrameRef.current = null
			}
			pendingPanRef.current = null
			if (suppressClickTimeoutRef.current) {
				clearTimeout(suppressClickTimeoutRef.current)
				suppressClickTimeoutRef.current = null
			}
			if (coordTimeoutRef.current) {
				clearTimeout(coordTimeoutRef.current)
				coordTimeoutRef.current = null
			}
		}
	}, [])

	const handleMouseMove = (e: React.MouseEvent) => {
		if (isPanning) {
			setHasMouseMoved(true)
			schedulePanUpdate({
				x: e.clientX - panStart.x,
				y: e.clientY - panStart.y
			})
		}

		// 计算鼠标在画布上的像素坐标
		if (hasPointerDevice && canvasRef.current && containerRef.current) {
			const canvas = canvasRef.current
			const container = containerRef.current
			const rect = container.getBoundingClientRect()

			// 获取鼠标在容器中的相对位置
			const mouseX = e.clientX - rect.left
			const mouseY = e.clientY - rect.top

			// 计算画布在容器中的位置（考虑缩放和平移）
			const currentZoom = zoomRef.current
			const currentPan = panRef.current

			// 画布原点在容器中的位置
			const canvasOriginX = currentPan.x + BOARD_WIDTH / 2
			const canvasOriginY = currentPan.y + BOARD_HEIGHT / 2

			// 鼠标相对于画布原点的位置
			const relX = mouseX - canvasOriginX
			const relY = mouseY - canvasOriginY

			// 转换为画布坐标系（考虑缩放）
			const boardX = relX / currentZoom + BOARD_WIDTH / 2
			const boardY = relY / currentZoom + BOARD_HEIGHT / 2

			// 检查是否在画布范围内
			if (
				boardX >= 0 &&
				boardX < BOARD_WIDTH &&
				boardY >= 0 &&
				boardY < BOARD_HEIGHT
			) {
				const newCoords = {
					x: Math.floor(boardX),
					y: Math.floor(boardY)
				}
				setMousePixelCoords(newCoords)

				// 显示坐标胶囊
				if (!isCoordVisible) {
					setIsCoordVisible(true)
				}
			} else {
				setMousePixelCoords(null)
				if (isCoordVisible) {
					setIsCoordVisible(false)
				}
			}
		}
	}

	const handleMouseUp = () => {
		setIsPanning(false)
		// Reset hasMouseMoved after a short delay to allow click event to fire
		setTimeout(() => setHasMouseMoved(false), 0)
	}

	const handleMouseLeave = () => {
		// 鼠标离开画布时触发退出动画
		if (isCoordVisible) {
			setIsCoordVisible(false)
		}
		// 延迟清除坐标以等待退出动画完成
		if (coordTimeoutRef.current) {
			clearTimeout(coordTimeoutRef.current)
		}
		coordTimeoutRef.current = setTimeout(() => {
			setMousePixelCoords(null)
		}, 250) // 与退出动画时长匹配
	}

	// Touch event handlers
	const getTouchDistance = (touches: React.TouchList) => {
		if (touches.length < 2) return 0
		const dx = touches[0].clientX - touches[1].clientX
		const dy = touches[0].clientY - touches[1].clientY
		return Math.sqrt(dx * dx + dy * dy)
	}

	const handleTouchStart = (e: React.TouchEvent) => {
		suppressClickRef.current = true
		if (suppressClickTimeoutRef.current) {
			clearTimeout(suppressClickTimeoutRef.current)
			suppressClickTimeoutRef.current = null
		}
		if (e.touches.length === 1) {
			// Single finger - prepare for pan or tap
			const touch = e.touches[0]
			const currentPan = panRef.current
			setTouchStartPos({ x: touch.clientX, y: touch.clientY })
			setPanStart({
				x: touch.clientX - currentPan.x,
				y: touch.clientY - currentPan.y
			})
			setIsTouchDragging(false)
		} else if (e.touches.length === 2) {
			// Two fingers - pinch zoom
			e.preventDefault()
			const distance = getTouchDistance(e.touches)
			setLastTouchDistance(distance)
			setIsTouchDragging(true)
		}
	}

	const handleTouchMove = (e: React.TouchEvent) => {
		if (e.touches.length === 1 && touchStartPos) {
			// Single finger pan
			const touch = e.touches[0]
			const moveDistance = Math.sqrt(
				Math.pow(touch.clientX - touchStartPos.x, 2) +
					Math.pow(touch.clientY - touchStartPos.y, 2)
			)

			// If moved more than 5px, consider it a drag
			if (moveDistance > 5) {
				setIsTouchDragging(true)
				schedulePanUpdate({
					x: touch.clientX - panStart.x,
					y: touch.clientY - panStart.y
				})
			}
		} else if (e.touches.length === 2 && lastTouchDistance) {
			// Two finger pinch zoom around midpoint of touches
			e.preventDefault()
			const distance = getTouchDistance(e.touches)
			const scale = distance / lastTouchDistance
			const el = containerRef.current
			if (el) {
				const rect = el.getBoundingClientRect()
				const midClientX = (e.touches[0].clientX + e.touches[1].clientX) / 2
				const midClientY = (e.touches[0].clientY + e.touches[1].clientY) / 2
				const anchorX = midClientX - rect.left
				const anchorY = midClientY - rect.top
				const target = clampZoom(zoom * scale)
				zoomAt(target, anchorX, anchorY)
			}
			setLastTouchDistance(distance)
		}
	}

	const handleTouchEnd = (e: React.TouchEvent) => {
		if (e.touches.length === 0) {
			// All fingers lifted
			if (!isTouchDragging && touchStartPos && canvasRef.current) {
				// This was a tap, not a drag - paint pixel
				const canvas = canvasRef.current
				const rect = canvas.getBoundingClientRect()
				const clickX = touchStartPos.x - rect.left
				const clickY = touchStartPos.y - rect.top
				const x = Math.floor(clickX / zoom)
				const y = Math.floor(clickY / zoom)

				if (x >= 0 && x < BOARD_WIDTH && y >= 0 && y < BOARD_HEIGHT) {
					if (!token) {
						toast.error('未设置 Token', {
							description: '请先获取或在设置中输入 Token'
						})
					} else if (!uid) {
						toast.error('未设置 UID', {
							description: '已输入 Token，但未填写 UID，请在设置面板中输入 UID'
						})
					} else {
						const color = selectedColor
						const r = Number.parseInt(color.slice(1, 3), 16)
						const g = Number.parseInt(color.slice(3, 5), 16)
						const b = Number.parseInt(color.slice(5, 7), 16)
						paintPixel(uid, token, r, g, b, x, y)
					}
				}
			}
			setTouchStartPos(null)
			setLastTouchDistance(null)
			setIsTouchDragging(false)
			suppressClickTimeoutRef.current = setTimeout(() => {
				suppressClickRef.current = false
				suppressClickTimeoutRef.current = null
			}, 250)
		} else if (e.touches.length === 1) {
			// One finger lifted during two-finger gesture
			setLastTouchDistance(null)
		}
	}

	// Heatmap animation loop - JIT 优化版本
	useEffect(() => {
		if (!isHeatmapEnabled) {
			if (heatmapAnimationRef.current) {
				cancelAnimationFrame(heatmapAnimationRef.current)
				heatmapAnimationRef.current = null
			}
			// Reset to normal rendering when disabled
			heatmapImageDataRef.current = null
			darkenedColorsRef.current = null
			renderBoard()
			return
		}

		// 当背景亮度改变时，清除缓存的暗化颜色
		darkenedColorsRef.current = null

		const canvas = canvasRef.current
		if (!canvas) return
		const ctx = canvas.getContext('2d')
		if (!ctx) return

		// 初始化复用的 ImageData（只创建一次）
		if (!heatmapImageDataRef.current) {
			heatmapImageDataRef.current = ctx.createImageData(
				BOARD_WIDTH,
				BOARD_HEIGHT
			)
		}

		// 预计算暗化后的颜色（使用可调的背景亮度）- 只在需要时计算一次
		if (!darkenedColorsRef.current) {
			const darkened = new Uint8Array(BOARD_WIDTH * BOARD_HEIGHT * 3)
			const boardData = boardDataRef.current
			const totalPixels = BOARD_WIDTH * BOARD_HEIGHT
			const brightnessFactor = heatmapBackgroundBrightness / 100
			// JIT 友好：单一循环，类型稳定，可预测分支
			for (let i = 0; i < totalPixels; i++) {
				const idx = i * 3
				const idx1 = idx + 1
				const idx2 = idx + 2
				// 使用位运算代替 Math.floor（更快，JIT 友好）
				darkened[idx] = (boardData[idx] * brightnessFactor) | 0
				darkened[idx1] = (boardData[idx1] * brightnessFactor) | 0
				darkened[idx2] = (boardData[idx2] * brightnessFactor) | 0
			}
			darkenedColorsRef.current = darkened
		}

		// JIT 优化：将热路径提取为独立函数，便于内联和优化
		// 单态函数：参数类型稳定，无隐藏类变化
		const fillWithDarkenedColors = (
			data32: Uint32Array,
			darkenedColors: Uint8Array,
			totalPixels: number
		): void => {
			// 循环展开：一次处理 4 个像素，减少循环开销
			const remainder = totalPixels % 4
			const loopEnd = totalPixels - remainder

			let i = 0
			// 主循环：4x 展开
			for (; i < loopEnd; i += 4) {
				const srcIdx0 = i * 3
				const srcIdx1 = srcIdx0 + 3
				const srcIdx2 = srcIdx1 + 3
				const srcIdx3 = srcIdx2 + 3

				// 手动展开，避免函数调用开销
				const r0 = darkenedColors[srcIdx0]
				const g0 = darkenedColors[srcIdx0 + 1]
				const b0 = darkenedColors[srcIdx0 + 2]
				data32[i] = 0xff000000 | (b0 << 16) | (g0 << 8) | r0

				const r1 = darkenedColors[srcIdx1]
				const g1 = darkenedColors[srcIdx1 + 1]
				const b1 = darkenedColors[srcIdx1 + 2]
				data32[i + 1] = 0xff000000 | (b1 << 16) | (g1 << 8) | r1

				const r2 = darkenedColors[srcIdx2]
				const g2 = darkenedColors[srcIdx2 + 1]
				const b2 = darkenedColors[srcIdx2 + 2]
				data32[i + 2] = 0xff000000 | (b2 << 16) | (g2 << 8) | r2

				const r3 = darkenedColors[srcIdx3]
				const g3 = darkenedColors[srcIdx3 + 1]
				const b3 = darkenedColors[srcIdx3 + 2]
				data32[i + 3] = 0xff000000 | (b3 << 16) | (g3 << 8) | r3
			}

			// 处理剩余像素
			for (; i < totalPixels; i++) {
				const srcIdx = i * 3
				const r = darkenedColors[srcIdx]
				const g = darkenedColors[srcIdx + 1]
				const b = darkenedColors[srcIdx + 2]
				data32[i] = 0xff000000 | (b << 16) | (g << 8) | r
			}
		}

		// JIT 优化：活跃像素处理函数（单态、类型稳定）
		const processActivePixels = (
			data32: Uint32Array,
			boardData: Uint8Array,
			timestamps: Map<number, number>,
			fadeDurationMs: number,
			now: number
		): void => {
			// 使用 forEach 而非 Array.from，减少中间数组分配
			timestamps.forEach((timestamp, pixelIdx) => {
				const elapsed = now - timestamp

				if (elapsed >= fadeDurationMs) {
					// 完全淡出，从 map 中删除
					timestamps.delete(pixelIdx)
					return
				}

				// 使用整数运算代替浮点（JIT 更友好）
				// brightness: 1.0 -> 0.3, 即 100% -> 30%
				const elapsedInt = elapsed | 0
				const durationInt = fadeDurationMs | 0
				// 线性插值：30 + (100-30) * (1 - elapsed/duration)
				// = 30 + 70 * (1 - elapsed/duration)
				const brightnessPercent =
					(30 + (70 * (durationInt - elapsedInt)) / durationInt) | 0

				const srcIdx = pixelIdx * 3
				// 使用位运算优化：(x * percent / 100) | 0
				const r = ((boardData[srcIdx] * brightnessPercent) / 100) | 0
				const g = ((boardData[srcIdx + 1] * brightnessPercent) / 100) | 0
				const b = ((boardData[srcIdx + 2] * brightnessPercent) / 100) | 0

				// 批量写入 32 位
				data32[pixelIdx] = 0xff000000 | (b << 16) | (g << 8) | r
			})
		}

		const totalPixels = BOARD_WIDTH * BOARD_HEIGHT
		const fadeDurationMs = heatmapFadeDuration * 1000

		const animate = (): void => {
			const now = Date.now()

			const imageData = heatmapImageDataRef.current!
			const data = imageData.data
			const boardData = boardDataRef.current
			const darkenedColors = darkenedColorsRef.current!
			const timestamps = pixelTimestampsRef.current

			// 使用 Uint32Array 进行 4 字节批量写入
			const data32 = new Uint32Array(data.buffer)

			// 1. 先用暗化颜色填充整个画布（JIT 优化版本）
			fillWithDarkenedColors(data32, darkenedColors, totalPixels)

			// 2. 处理活跃像素（JIT 优化版本）
			processActivePixels(data32, boardData, timestamps, fadeDurationMs, now)

			// 3. 一次性提交到 canvas
			ctx.putImageData(imageData, 0, 0)

			heatmapAnimationRef.current = requestAnimationFrame(animate)
		}

		animate()

		return () => {
			if (heatmapAnimationRef.current) {
				cancelAnimationFrame(heatmapAnimationRef.current)
				heatmapAnimationRef.current = null
			}
		}
	}, [
		isHeatmapEnabled,
		heatmapFadeDuration,
		heatmapBackgroundBrightness,
		renderBoard
	])

	// 当画板数据变化时，重新计算暗化颜色缓存 - JIT 优化版本
	useEffect(() => {
		if (isHeatmapEnabled && darkenedColorsRef.current) {
			const darkened = new Uint8Array(BOARD_WIDTH * BOARD_HEIGHT * 3)
			const boardData = boardDataRef.current
			const totalPixels = BOARD_WIDTH * BOARD_HEIGHT
			const brightnessFactor = heatmapBackgroundBrightness / 100

			// JIT 友好：展开循环，减少分支
			const remainder = totalPixels % 4
			const loopEnd = totalPixels - remainder

			let i = 0
			// 4x 循环展开
			for (; i < loopEnd; i += 4) {
				const idx0 = i * 3
				const idx1 = idx0 + 3
				const idx2 = idx1 + 3
				const idx3 = idx2 + 3

				// 使用位运算代替 Math.floor，JIT 更容易优化
				darkened[idx0] = (boardData[idx0] * brightnessFactor) | 0
				darkened[idx0 + 1] = (boardData[idx0 + 1] * brightnessFactor) | 0
				darkened[idx0 + 2] = (boardData[idx0 + 2] * brightnessFactor) | 0

				darkened[idx1] = (boardData[idx1] * brightnessFactor) | 0
				darkened[idx1 + 1] = (boardData[idx1 + 1] * brightnessFactor) | 0
				darkened[idx1 + 2] = (boardData[idx1 + 2] * brightnessFactor) | 0

				darkened[idx2] = (boardData[idx2] * brightnessFactor) | 0
				darkened[idx2 + 1] = (boardData[idx2 + 1] * brightnessFactor) | 0
				darkened[idx2 + 2] = (boardData[idx2 + 2] * brightnessFactor) | 0

				darkened[idx3] = (boardData[idx3] * brightnessFactor) | 0
				darkened[idx3 + 1] = (boardData[idx3 + 1] * brightnessFactor) | 0
				darkened[idx3 + 2] = (boardData[idx3 + 2] * brightnessFactor) | 0
			}

			// 处理剩余像素
			for (; i < totalPixels; i++) {
				const idx = i * 3
				darkened[idx] = (boardData[idx] * brightnessFactor) | 0
				darkened[idx + 1] = (boardData[idx + 1] * brightnessFactor) | 0
				darkened[idx + 2] = (boardData[idx + 2] * brightnessFactor) | 0
			}

			darkenedColorsRef.current = darkened
		}
	}, [baseUrl, isHeatmapEnabled, heatmapBackgroundBrightness])

	// Keyboard shortcuts and wheel zoom
	useEffect(() => {
		const onKeyDown = (e: KeyboardEvent) => {
			// 支持 + 和 -，兼容不同键盘布局
			if (e.key === '+' || e.key === '=') {
				const target = clampZoom(zoomRef.current * 1.1)
				zoomAtViewportCenter(target)
			}
			if (e.key === '-') {
				const target = clampZoom(zoomRef.current / 1.1)
				zoomAtViewportCenter(target)
			}
		}

		window.addEventListener('keydown', onKeyDown)

		return () => {
			window.removeEventListener('keydown', onKeyDown)
		}
	}, [zoomAtViewportCenter, clampZoom])

	useEffect(() => {
		const el = containerRef.current
		if (!el) return

		const onWheel = (e: WheelEvent) => {
			// 当按住 Ctrl 时，避免与浏览器缩放冲突
			if (e.ctrlKey) return

			e.preventDefault()
			e.stopPropagation()

			// deltaY < 0 -> 放大, deltaY > 0 -> 缩小
			const factor = e.deltaY < 0 ? 1.05 : 0.95
			const target = clampZoom(zoomRef.current * factor)
			// 以视口中心缩放
			const rect = el.getBoundingClientRect()
			const anchorX = rect.width / 2
			const anchorY = rect.height / 2
			zoomAt(target, anchorX, anchorY)
		}

		el.addEventListener('wheel', onWheel, { passive: false })

		return () => {
			el.removeEventListener('wheel', onWheel)
		}
	}, [zoomAt, clampZoom])

	return (
		<div className="relative h-full w-full flex flex-col">
			{/* 悬浮标题和状态胶囊 - 左上角 */}
                        <div
                                className="absolute z-30 bg-card/70 backdrop-blur-sm rounded-xl border border-border shadow-lg px-5 py-3 pointer-events-none"
                                style={{
                                        top: 'calc(1rem + env(safe-area-inset-top))',
                                        left: 'calc(1rem + env(safe-area-inset-left))'
                                }}
                        >
				<div className="flex flex-col items-start gap-2">
					<h1 className="text-xl font-bold text-foreground whitespace-nowrap text-left">
						Paintboard FE
					</h1>
					<div className="inline-flex items-center gap-2">
						<div
							className={cn(
								'h-2 w-2 rounded-full',
								wsStatus === 'connected' &&
									'bg-emerald-500 animate-green-breath',
								wsStatus === 'connecting' && 'bg-chart-4 animate-pulse',
								wsStatus === 'disconnected' && 'bg-destructive'
							)}
						/>
						<span className="text-xs text-muted-foreground font-mono text-left">
							{wsStatus === 'connected' && 'Connected'}
							{wsStatus === 'connecting' && 'Connecting...'}
							{wsStatus === 'disconnected' && 'Disconnected'}
						</span>
					</div>
				</div>
			</div>

			<div className="flex-1 flex overflow-hidden">
				{!baseUrl ? (
					<div className="flex-1 flex items-center justify-center bg-secondary/50">
						<Card className="p-8 max-w-md text-center space-y-4">
							<h2 className="text-xl font-bold text-foreground">
								欢迎使用 Paintboard FE
							</h2>
							<p className="text-muted-foreground">
								请先在右侧设置面板中配置 Base URL 以连接到绘版服务器。
							</p>
							<p className="text-sm text-muted-foreground">
								例如:{' '}
								<code className="bg-muted px-2 py-1 rounded">
									https://paintboard.luogu.me
								</code>
							</p>
						</Card>
					</div>
				) : (
					<div
						ref={containerRef}
						className="flex-1 relative overflow-hidden bg-secondary/50"
						onMouseDown={handleMouseDown}
						onMouseMove={handleMouseMove}
						onMouseUp={handleMouseUp}
						onMouseLeave={handleMouseLeave}
						onTouchStart={handleTouchStart}
						onTouchMove={handleTouchMove}
						onTouchEnd={handleTouchEnd}
						style={{
							cursor: isPanning ? 'grabbing' : 'crosshair',
							touchAction: 'none' // Prevent default touch behaviors
						}}
					>
						<canvas
							ref={canvasRef}
							width={BOARD_WIDTH}
							height={BOARD_HEIGHT}
							onClick={handleCanvasClick}
							className="absolute"
							style={{
								transform: `translate(${pan.x}px, ${pan.y}px) scale(${zoom})`,
								// 使用画布中心作为缩放锚点
								transformOrigin: `${BOARD_WIDTH / 2}px ${BOARD_HEIGHT / 2}px`,
								imageRendering: 'pixelated',
								transition:
									isPanning || isTouchDragging
										? 'none'
										: 'transform 180ms cubic-bezier(0.2, 0.8, 0.2, 1)'
							}}
						/>

                                                <div
                                                        className="absolute"
                                                        onMouseDown={stopOverlayPropagation}
                                                        onMouseUp={stopOverlayPropagation}
                                                        onPointerDown={stopOverlayPropagation}
                                                        onPointerUp={stopOverlayPropagation}
                                                        onTouchStart={stopOverlayPropagation}
                                                        onTouchMove={stopOverlayPropagation}
                                                        onTouchEnd={stopOverlayPropagation}
                                                        onTouchCancel={stopOverlayPropagation}
                                                        onClick={stopOverlayPropagation}
                                                        style={{
                                                                cursor: 'default',
                                                                touchAction: 'manipulation',
                                                                right: 'calc(1.5rem + env(safe-area-inset-right))',
                                                                bottom: 'calc(1.5rem + env(safe-area-inset-bottom))'
                                                        }}
                                                >
							<div className="relative">
								{/* 纵向滑条面板 - 常驻显示 */}
                                                                <div
                                                                        className="absolute bottom-full right-0 mb-2 bg-card/80 backdrop-blur-sm rounded-lg border border-border shadow-lg py-3 flex flex-col items-center gap-2 w-9"
                                                                        onMouseDown={stopOverlayPropagation}
                                                                        onMouseMove={stopOverlayPropagation}
                                                                        onMouseUp={stopOverlayPropagation}
                                                                        onPointerDown={stopOverlayPropagation}
                                                                        onPointerMove={stopOverlayPropagation}
                                                                        onPointerUp={stopOverlayPropagation}
                                                                        onTouchStart={stopOverlayPropagation}
                                                                        onTouchMove={stopOverlayPropagation}
                                                                        onTouchEnd={stopOverlayPropagation}
                                                                        onTouchCancel={stopOverlayPropagation}
                                                                        onClick={stopOverlayPropagation}
                                                                        style={{ cursor: 'default', touchAction: 'manipulation' }}
                                                                >
									<p className="text-xs font-mono text-muted-foreground leading-none">
										{(zoom * 100).toFixed(0)}%
									</p>
									<Slider.Root
										orientation="vertical"
										className="relative flex items-center justify-center select-none touch-none h-40 w-9 cursor-pointer"
										value={[zoom]}
										min={MIN_ZOOM}
										max={MAX_ZOOM}
										step={0.1}
										onValueChange={val => zoomAtViewportCenter(val[0])}
									>
										<Slider.Track className="relative bg-muted w-1.5 h-full rounded-full cursor-pointer">
											<Slider.Range className="absolute bg-primary w-full rounded-full" />
										</Slider.Track>
										<Slider.Thumb className="block w-4 h-4 bg-background border-2 border-primary rounded-full shadow-md hover:scale-110 transition-transform cursor-grab active:cursor-grabbing" />
									</Slider.Root>
								</div>

								{/* 右下角按钮栈 */}
								<div className="flex flex-col items-center gap-2">
									{/* Heatmap toggle - independent button */}
									<Button
										size="icon"
										variant={isHeatmapEnabled ? 'default' : 'secondary'}
										onClick={() => setIsHeatmapEnabled(!isHeatmapEnabled)}
										className={cn(
											'shadow-lg backdrop-blur-sm',
											isHeatmapEnabled
												? 'bg-orange-500/90 hover:bg-orange-600/90'
												: 'bg-secondary/80 hover:bg-secondary/90'
										)}
										title={isHeatmapEnabled ? '关闭热力视图' : '开启热力视图'}
									>
										<Flame
											className={cn(
												'h-4 w-4',
												isHeatmapEnabled ? 'text-white' : 'text-foreground'
											)}
										/>
									</Button>

									{/* Color picker button */}
									<div className="relative">
										<input
											type="color"
											value={selectedColor}
											onChange={e => setSelectedColor(e.target.value)}
											className="absolute inset-0 opacity-0 cursor-pointer w-full h-full"
											title="选择颜色"
										/>
										<Button
											size="icon"
											variant="secondary"
											className="pointer-events-none shadow-lg backdrop-blur-sm bg-secondary/80"
											title="选择颜色"
										>
											<div
												className="w-4 h-4 rounded border border-border/50"
												style={{ backgroundColor: selectedColor }}
											/>
										</Button>
									</div>

									<Button
										size="icon"
										variant="secondary"
										onClick={handleZoomIn}
										className="shadow-lg backdrop-blur-sm bg-secondary/80 hover:bg-secondary/90"
									>
										<ZoomIn className="h-4 w-4" />
									</Button>
									<Button
										size="icon"
										variant="secondary"
										onClick={handleZoomOut}
										className="shadow-lg backdrop-blur-sm bg-secondary/80 hover:bg-secondary/90"
									>
										<ZoomOut className="h-4 w-4" />
									</Button>
									<Button
										size="icon"
										variant="secondary"
										onClick={handleResetView}
										className="shadow-lg backdrop-blur-sm bg-secondary/80 hover:bg-secondary/90"
									>
										<Maximize2 className="h-4 w-4" />
									</Button>
									<Button
										size="icon"
										variant="secondary"
										onClick={() => setIsSidebarCollapsed(!isSidebarCollapsed)}
										className="shadow-lg backdrop-blur-sm bg-secondary/80 hover:bg-secondary/90"
										title={isSidebarCollapsed ? '展开侧边栏' : '折叠侧边栏'}
									>
										{isSidebarCollapsed ? (
											<PanelRightOpen className="h-4 w-4" />
										) : (
											<PanelRightClose className="h-4 w-4" />
										)}
									</Button>
								</div>
							</div>
						</div>

						{/* 左下角鼠标坐标显示 */}
						{hasPointerDevice && mousePixelCoords && (
							<div
								className={cn(
									'absolute bottom-3 left-3 z-50 bg-card/70 backdrop-blur-sm rounded-lg border border-border shadow-lg px-3 py-1.5',
									isCoordVisible
										? 'animate-slide-up-in'
										: 'animate-slide-down-out'
								)}
							>
								<div className="flex items-center gap-2">
									<span className="text-[10px] text-muted-foreground font-mono">
										X:
									</span>
									<span className="text-xs font-mono text-foreground font-semibold min-w-[2.5ch] text-right">
										{mousePixelCoords.x}
									</span>
									<span className="text-[10px] text-muted-foreground">|</span>
									<span className="text-[10px] text-muted-foreground font-mono">
										Y:
									</span>
									<span className="text-xs font-mono text-foreground font-semibold min-w-[2.5ch] text-right">
										{mousePixelCoords.y}
									</span>
								</div>
							</div>
						)}
					</div>
				)}

				<aside
					className={cn(
						'border-l border-border bg-card overflow-y-auto transition-all duration-300 ease-in-out',
						isSidebarCollapsed ? 'w-0 p-0' : 'w-80 p-6'
					)}
				>
					<div
						className={cn(
							'space-y-6 transition-opacity duration-300',
							isSidebarCollapsed
								? 'opacity-0 pointer-events-none'
								: 'opacity-100'
						)}
					>
						{/* 主题切换 */}
						<Card className="p-4">
							<div className="flex items-center justify-between">
								<h3 className="font-semibold text-foreground">主题</h3>
								<ThemeToggle />
							</div>
						</Card>

						<Card className="p-4 space-y-3">
							<div className="flex items-center gap-2">
								<Palette className="h-4 w-4 text-primary" />
								<h3 className="font-semibold text-foreground">颜色选择</h3>
							</div>
							<div className="space-y-2">
								<div className="flex items-center gap-3">
									<input
										type="color"
										value={selectedColor}
										onChange={e => setSelectedColor(e.target.value)}
										className="h-12 w-12 rounded cursor-pointer border-2 border-border"
									/>
									<div className="flex-1">
										<Input
											value={selectedColor}
											onChange={e => setSelectedColor(e.target.value)}
											className="font-mono"
											placeholder="#ff0000"
										/>
									</div>
								</div>
							</div>
						</Card>

						<Card className="p-4 space-y-4">
							<h3 className="font-semibold text-foreground">设置</h3>

							<div className="space-y-2">
								<Label
									htmlFor="baseUrl"
									className="text-sm text-muted-foreground"
								>
									Base URL *
								</Label>
								<div className="flex gap-2">
									<Input
										id="baseUrl"
										value={baseUrlInput}
										onChange={e => setBaseUrlInput(e.target.value)}
										placeholder="https://paintboard.luogu.me"
										className="font-mono flex-1"
									/>
									<Button
										size="icon"
										onClick={handleApplyBaseUrl}
										disabled={baseUrlInput === baseUrl || isApplyingBaseUrl}
										variant={
											baseUrlInput !== baseUrl && !isApplyingBaseUrl
												? 'default'
												: 'outline'
										}
									>
										<Check className="h-4 w-4" />
									</Button>
								</div>
								<p className="text-xs text-muted-foreground">
									点击确认按钮应用更改
								</p>
							</div>

							<div className="border-t border-border pt-4 space-y-4">
								<h4 className="font-semibold text-foreground text-sm flex items-center gap-2">
									<Flame className="h-4 w-4 text-orange-500" />
									热力视图设置
								</h4>

								<div className="space-y-2">
									<div className="flex items-center justify-between">
										<Label
											htmlFor="heatmapDuration"
											className="text-sm text-muted-foreground"
										>
											完全变暗时间（秒）
										</Label>
										<span className="text-sm font-mono text-foreground">
											{heatmapFadeDuration}s
										</span>
									</div>
									<Suspense fallback={null}>
										<HeatmapSlider
											id="heatmapDuration"
											value={heatmapFadeDuration}
											min={1}
											max={30}
											step={1}
											onChange={v => setHeatmapFadeDuration(v)}
										/>
									</Suspense>
									<p className="text-xs text-muted-foreground">
										新画出的像素从正常亮度渐变到背景亮度所需时间
									</p>
								</div>

								<div className="space-y-2">
									<div className="flex items-center justify-between">
										<Label
											htmlFor="heatmapBrightness"
											className="text-sm text-muted-foreground"
										>
											背景亮度（%）
										</Label>
										<span className="text-sm font-mono text-foreground">
											{heatmapBackgroundBrightness}%
										</span>
									</div>
									<Suspense fallback={null}>
										<HeatmapSlider
											id="heatmapBrightness"
											value={heatmapBackgroundBrightness}
											min={0}
											max={100}
											step={5}
											onChange={v => setHeatmapBackgroundBrightness(v)}
										/>
									</Suspense>
									<p className="text-xs text-muted-foreground">
										热力视图中背景（未更新像素）的亮度
									</p>
								</div>
							</div>

							<div className="border-t border-border pt-4 space-y-4">
								<h4 className="font-semibold text-foreground text-sm">
									Token 设置
								</h4>

								<div className="space-y-2">
									<Label
										htmlFor="uid"
										className="text-sm text-muted-foreground"
									>
										UID
									</Label>
									<Input
										id="uid"
										type="number"
										value={uid ?? ''}
										onChange={e => {
											const value = e.target.value
											setUid(value === '' ? null : Number.parseInt(value, 10))
										}}
										placeholder="输入你的 UID"
										className="font-mono"
									/>
								</div>

								<div className="space-y-2">
									<Label
										htmlFor="accessKey"
										className="text-sm text-muted-foreground"
									>
										Access Key
									</Label>
									<Input
										id="accessKey"
										type="password"
										value={accessKey}
										onChange={e => setAccessKey(e.target.value)}
										placeholder="输入你的 Access Key"
										className="font-mono"
									/>
								</div>

								<Button onClick={handleGetToken} className="w-full">
									获取 Token
								</Button>

								<div className="space-y-2">
									<Label
										htmlFor="token"
										className="text-sm text-muted-foreground"
									>
										当前 Token
									</Label>
									<Input
										id="token"
										value={token}
										onChange={e => setToken(e.target.value)}
										placeholder="在此粘贴或输入已有的 Token（UUID 格式）"
										className="font-mono text-xs"
									/>
								</div>
							</div>
						</Card>

						<Card className="p-4 space-y-3">
							<h3 className="font-semibold text-foreground">使用说明</h3>
							<ul className="space-y-2 text-sm text-muted-foreground">
								<li className="flex items-start gap-2">
									<span className="text-primary">•</span>
									<span>点击或轻触画布进行绘画</span>
								</li>
								<li className="flex items-start gap-2">
									<span className="text-primary">•</span>
									<span>按住 Shift + 拖动或中键拖动平移画布</span>
								</li>
								<li className="flex items-start gap-2">
									<span className="text-primary">•</span>
									<span>手机上：单指拖动画布，双指捏合缩放</span>
								</li>
								<li className="flex items-start gap-2">
									<span className="text-primary">•</span>
									<span>使用右侧按钮缩放画布</span>
								</li>
								<li className="flex items-start gap-2">
									<span className="text-primary">•</span>
									<span>点击火焰按钮开启热力视图，查看绘制活跃区域</span>
								</li>
								<li className="flex items-start gap-2">
									<span className="text-primary">•</span>
									<span>需要 Token 才能绘画</span>
								</li>
							</ul>
						</Card>

						<Card className="p-4 space-y-2">
							<h3 className="font-semibold text-foreground">绘版信息</h3>
							<div className="space-y-1 text-sm font-mono text-muted-foreground">
								<p>
									尺寸: {BOARD_WIDTH} × {BOARD_HEIGHT}
								</p>
								<p>像素总数: {(BOARD_WIDTH * BOARD_HEIGHT).toLocaleString()}</p>
							</div>
						</Card>
					</div>
				</aside>
			</div>

			<div
				onTouchStartCapture={stopOverlayPropagation}
				onTouchMoveCapture={stopOverlayPropagation}
				onTouchEndCapture={stopOverlayPropagation}
				onTouchCancelCapture={stopOverlayPropagation}
				onPointerDownCapture={stopOverlayPropagation}
				onPointerMoveCapture={stopOverlayPropagation}
				onPointerUpCapture={stopOverlayPropagation}
				onPointerCancelCapture={stopOverlayPropagation}
				onMouseDownCapture={stopOverlayPropagation}
				onMouseUpCapture={stopOverlayPropagation}
			>
				<Toaster />
			</div>
		</div>
	)
}
