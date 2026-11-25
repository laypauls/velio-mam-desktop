import { useEffect, useRef, useState } from "react"
import type { FrameJob, FrameSource } from "./useFrameBuffers"

const VERTEX = `#version 300 es
out vec2 uv;
void main() {
  vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
  uv = vec2(p.x, 1.0 - p.y);
  gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}`
// bgr0: 바이트 순서 B,G,R,0 → RGBA 텍스처로 올리면 .r=B .g=G .b=R. 알파(0)는 무시하고 1 로 둔다
const FRAGMENT = `#version 300 es
precision mediump float;
in vec2 uv;
uniform sampler2D tex;
out vec4 o;
void main() {
  vec4 t = texture(tex, uv);
  o = vec4(t.b, t.g, t.r, 1.0);
}`

export type GlStatus = "ok" | "unsupported"

/** 한 컨텍스트의 GL 자원 — 프로그램·텍스처. 컨텍스트가 복구되면 다시 만든다 */
interface GlState {
  texture: WebGLTexture
  /** 텍스처에 할당된 크기 (0 = 아직 없음) */
  w: number
  h: number
}

function compile(gl: WebGL2RenderingContext, type: number, source: string): WebGLShader | null {
  const shader = gl.createShader(type)
  if (!shader) return null
  gl.shaderSource(shader, source)
  gl.compileShader(shader)
  return gl.getShaderParameter(shader, gl.COMPILE_STATUS) ? shader : null
}

function setup(gl: WebGL2RenderingContext): GlState | null {
  const vs = compile(gl, gl.VERTEX_SHADER, VERTEX)
  const fs = compile(gl, gl.FRAGMENT_SHADER, FRAGMENT)
  const program = gl.createProgram()
  const texture = gl.createTexture()
  if (!vs || !fs || !program || !texture) return null
  gl.attachShader(program, vs)
  gl.attachShader(program, fs)
  gl.linkProgram(program)
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) return null
  gl.useProgram(program)
  gl.bindTexture(gl.TEXTURE_2D, texture)
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR)
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR)
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE)
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE)
  gl.pixelStorei(gl.UNPACK_ALIGNMENT, 4)
  return { texture, w: 0, h: 0 }
}

/**
 * 프레임 1장을 올려 그린다. 크기가 바뀐 프레임(새 에포크의 첫 프레임)이면 같은 작업 안에서 캔버스 크기·뷰포트·텍스처를 바꾸고 바로 그린다
 * — 캔버스 크기를 바꾸면 내용이 지워지므로 그리기와 분리하면 검은 화면이 깜빡인다 (계약 §3.6).
 * 업로드는 `texSubImage2D` (동기 복사)라 반환 직후 슬롯을 돌려줘도 된다.
 */
function drawFrame(gl: WebGL2RenderingContext, canvas: HTMLCanvasElement, state: GlState, job: FrameJob): void {
  const { frame, buffer } = job
  const { w, h, stride } = frame
  if (state.w !== w || state.h !== h) {
    canvas.width = w
    canvas.height = h
    gl.viewport(0, 0, w, h)
    gl.bindTexture(gl.TEXTURE_2D, state.texture)
    gl.pixelStorei(gl.UNPACK_ROW_LENGTH, 0)
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, null)
    state.w = w
    state.h = h
  }
  gl.bindTexture(gl.TEXTURE_2D, state.texture)
  gl.pixelStorei(gl.UNPACK_ROW_LENGTH, stride / 4)
  gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array(buffer, 0, stride * h))
  gl.drawArrays(gl.TRIANGLES, 0, 3)
}

/**
 * 호스트가 보낸 BGR0 프레임을 WebGL2 로 캔버스에 그린다 (PoC `poc_mpv.html` 이식).
 * rAF 는 그릴 프레임이 생겼을 때만 예약한다 — 정지 중에는 돌지 않는다. 언마운트 시 rAF 를 취소한다.
 * 컨텍스트가 사라지면(`webglcontextlost`) 그리기를 멈추되 프레임은 계속 돌려주고, 복구되면 자원을 다시 만든 뒤 `onRestored` 를 부른다
 * (마지막 프레임은 GPU 와 함께 사라지므로 호출부가 호스트에 다시 그리기를 요청한다).
 * WebGL2 를 쓸 수 없으면 `"unsupported"`.
 */
export function useWebglRenderer(canvas: HTMLCanvasElement | null, source: FrameSource, onRestored?: () => void): GlStatus {
  const [status, setStatus] = useState<GlStatus>("ok")
  const restored = useRef(onRestored)
  restored.current = onRestored
  useEffect(() => {
    if (!canvas) return
    const gl = canvas.getContext("webgl2", { alpha: false, antialias: false, depth: false, stencil: false, desynchronized: false })
    const initial = gl ? setup(gl) : null
    if (!gl || !initial) {
      setStatus("unsupported")
      return
    }
    setStatus("ok")
    let state: GlState | null = initial
    let raf = 0
    const frame = () => {
      raf = 0
      const job = source.takeDraw()
      if (!job) return
      try {
        if (state) drawFrame(gl, canvas, state, job)
      } finally {
        source.drawn(job) // 그리지 못했어도 슬롯은 돌려준다
      }
    }
    const request = () => {
      if (raf === 0) raf = requestAnimationFrame(frame)
    }
    const onLost = (e: Event) => {
      e.preventDefault()
      state = null
    }
    const onRestore = () => {
      state = setup(gl)
      if (state) restored.current?.()
    }
    canvas.addEventListener("webglcontextlost", onLost)
    canvas.addEventListener("webglcontextrestored", onRestore)
    const unsubscribe = source.subscribe(request)
    request()
    return () => {
      unsubscribe()
      if (raf !== 0) cancelAnimationFrame(raf)
      canvas.removeEventListener("webglcontextlost", onLost)
      canvas.removeEventListener("webglcontextrestored", onRestore)
    }
  }, [canvas, source])
  return status
}
