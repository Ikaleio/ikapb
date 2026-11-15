;(function (global) {
	'use strict'

	const existingFactory = global.Module
	if (typeof existingFactory === 'function') {
		global.ZstdCodecModuleFactory = existingFactory
		try {
			delete global.Module
		} catch (_) {
			global.Module = existingFactory
		}
	}

	const constants = {
		STREAMING_DEFAULT_BUFFER_SIZE: 512 * 1024
	}

	class ArrayBufferHelper {
		static transfer(oldBuffer, newCapacity) {
			const bytes = new Uint8Array(new ArrayBuffer(newCapacity))
			bytes.set(new Uint8Array(oldBuffer.slice(0, newCapacity)))
			return bytes.buffer
		}
	}

	function createZstd(binding) {
		const withBindingInstance = (instance, callback) => {
			try {
				return callback(instance)
			} finally {
				instance.delete()
			}
		}

		const withCppVector = callback => {
			const vector = new binding.VectorU8()
			return withBindingInstance(vector, callback)
		}

		class ArrayBufferSink {
			constructor(initialSize) {
				this._buffer = new ArrayBuffer(initialSize)
				this._array = new Uint8Array(this._buffer)
				this._offset = 0
			}

			concat(array) {
				if (!array || array.length === 0) {
					return
				}
				if (array.length + this._offset > this._array.length) {
					this._grow(Math.max(this._array.length, array.length) * 2)
				}
				this._array.set(array, this._offset)
				this._offset += array.length
			}

			array() {
				const buffer = ArrayBufferHelper.transfer(this._buffer, this._offset)
				return new Uint8Array(buffer)
			}

			_grow(newSize) {
				this._buffer = ArrayBufferHelper.transfer(this._buffer, newSize)
				this._array = new Uint8Array(this._buffer)
			}
		}

		class Streaming {
			decompress(compressedBytes, sizeHint) {
				if (!compressedBytes) {
					return null
				}
				return withBindingInstance(new binding.ZstdDecompressStreamBinding(), stream => {
					const initialSize =
						sizeHint && sizeHint > 0
							? sizeHint
							: this._estimateContentSize(compressedBytes)
					const sink = new ArrayBufferSink(initialSize)
					const callback = decompressed => {
						sink.concat(decompressed)
					}

					if (!stream.begin()) return null
					if (!stream.transform(compressedBytes, callback)) return null
					if (!stream.end(callback)) return null

					return sink.array()
				})
			}

			decompressUsingDict(compressedBytes, sizeHint, ddict) {
				if (!compressedBytes || !ddict) {
					return null
				}
				return withBindingInstance(new binding.ZstdDecompressStreamBinding(), stream => {
					const initialSize =
						sizeHint && sizeHint > 0
							? sizeHint
							: this._estimateContentSize(compressedBytes)
					const sink = new ArrayBufferSink(initialSize)
					const callback = decompressed => {
						sink.concat(decompressed)
					}

					if (!stream.beginUsingDict(ddict.get())) return null
					if (!stream.transform(compressedBytes, callback)) return null
					if (!stream.end(callback)) return null

					return sink.array()
				})
			}

			_estimateContentSize(compressedBytes) {
				return compressedBytes.length * 4
			}
		}

		class ZstdDecompressionDict {
			constructor(dictBytes) {
				this.binding = new binding.createDecompressionDict(dictBytes)
			}

			get() {
				return this.binding
			}

			close() {
				if (this.binding) {
					this.binding.delete()
					this.binding = null
				}
			}

			delete() {
				this.close()
			}
		}

		return {
			Streaming,
			Dict: {
				Decompression: ZstdDecompressionDict
			}
		}
	}

	let readyPromise = null

	function ensureReady() {
		if (!readyPromise) {
			const factory = global.ZstdCodecModuleFactory
			if (typeof factory !== 'function') {
				return Promise.reject(new Error('Zstd codec module is unavailable'))
			}

			readyPromise = new Promise((resolve, reject) => {
				let resolved = false
				const moduleInstance = factory({
					onRuntimeInitialized() {
						try {
							const zstd = createZstd(moduleInstance)
							resolved = true
							resolve(zstd)
						} catch (err) {
							reject(err)
						}
					}
				})

				if (moduleInstance && typeof moduleInstance.onAbort === 'function') {
					const originalAbort = moduleInstance.onAbort
					moduleInstance.onAbort = msg => {
						try {
							if (!resolved) {
								reject(new Error(msg || 'Zstd module aborted'))
								resolved = true
							}
						} finally {
							originalAbort.call(moduleInstance, msg)
						}
					}
				}
			})
		}
		return readyPromise
	}

	global.ZstdCodecBrowser = {
		ensureReady
	}
})(typeof window !== 'undefined' ? window : self)
