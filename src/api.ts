export interface ApiProblem {
  code: string
  message: string
  fieldPath?: string | null
}

const configuredBase = (import.meta.env.VITE_API_BASE_URL as string | undefined)?.replace(/\/$/, '') ?? ''

export function apiUrl(path: string): string {
  return `${configuredBase}${path}`
}

export async function apiRequest<T>(path: string, init?: RequestInit): Promise<T> {
  const headers = new Headers(init?.headers)
  if (init?.body && !headers.has('content-type')) headers.set('content-type', 'application/json')
  let response: Response
  try {
    response = await fetch(apiUrl(path), { ...init, headers })
  } catch (reason) {
    if (reason instanceof DOMException && reason.name === 'AbortError') throw reason
    throw new Error(`API_UNAVAILABLE: 无法连接权威模拟服务。${reason instanceof Error ? ` ${reason.message}` : ''}`)
  }
  if (!response.ok) {
    let problem: ApiProblem | null = null
    try {
      problem = await response.json() as ApiProblem
    } catch {
      // Preserve the HTTP status when an intermediary did not return the API problem format.
    }
    throw new Error(`${problem?.code ?? `HTTP_${response.status}`}: ${problem?.message ?? response.statusText}`)
  }
  if (response.status === 204) return undefined as T
  return response.json() as Promise<T>
}
