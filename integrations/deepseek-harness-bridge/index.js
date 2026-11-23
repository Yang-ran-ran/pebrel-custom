import { spawn } from 'node:child_process'
import { isAbsolute } from 'node:path'

export const name = 'pebrel-deepseek-harness-bridge'

const EVENT_MAP = new Map([
  ['user/message', 'UserPromptSubmit'],
  ['tool/result', 'PostToolUse'],
  ['turn/end', 'Stop'],
])

export function toPebrelPayload(session, event) {
  if (!session || !event || typeof event.type !== 'string') return undefined
  const hookEvent = EVENT_MAP.get(event.type)
  if (!hookEvent) return undefined

  // Harness event identity is the deduplication and ordering boundary in Pebrel.
  // Never manufacture "undefined" IDs or serialize NaN timestamps as null.
  if (session.id == null || String(session.id).trim() === '') return undefined
  if (!Number.isSafeInteger(event.seq) || event.seq < 0) return undefined
  if (!Number.isSafeInteger(event.time) || event.time < 0) return undefined

  const data = event.data ?? {}
  const payload = {
    hook_event_name: hookEvent,
    session_id: String(session.id),
    event_id: `dsh-${String(session.id)}-${event.seq}`,
    bridge_sequence: Number(event.seq),
    occurred_at_ms: Number(event.time),
    source: 'deepseek-harness',
  }

  // cwd is a routing hint for Pebrel when this Desktop process was not launched
  // from a specific Pane. Never substitute the Desktop install directory.
  if (
    typeof session.cwd === 'string'
    && (isAbsolute(session.cwd) || /^[A-Za-z]:[\\/]/.test(session.cwd) || /^\\\\/.test(session.cwd))
  ) {
    payload.cwd = session.cwd
  }

  if (event.type === 'tool/result') {
    payload.tool_name = typeof data.message?.name === 'string' ? data.message.name : 'unknown'
  }

  return payload
}

function forward(helperPath, session, event) {
  if (!isAbsolute(helperPath)) return Promise.resolve()
  if (!process.env.PEBREL_NOTIFY_PIPE && !process.env.NEBULA_NOTIFY_PIPE) return Promise.resolve()

  const payload = toPebrelPayload(session, event)
  if (!payload) return Promise.resolve()

  return new Promise(resolve => {
    const child = spawn(helperPath, ['deepseek-harness'], {
      env: process.env,
      stdio: ['pipe', 'ignore', 'ignore'],
      windowsHide: true,
    })
    child.once('error', resolve)
    child.once('close', resolve)
    child.stdin.on('error', () => {})
    child.stdin.end(JSON.stringify(payload))
  })
}

export function apply(ctx, config = {}) {
  const helperPath = typeof config.helperPath === 'string' ? config.helperPath : ''
  if (!isAbsolute(helperPath)) {
    ctx.logger.warn('pebrel-deepseek-harness-bridge: Pebrel hook helper path is unavailable; bridge is dormant')
    return
  }

  const queues = new Map()
  ctx.on('session/event', (session, event) => {
    // Cordis listeners must never throw back into the Desktop event dispatcher.
    // A malformed event has no stable queue identity and cannot be routed.
    if (session?.id == null || String(session.id).trim() === '' || !event || typeof event.type !== 'string') {
      return
    }
    const id = String(session.id)
    const previous = queues.get(id) ?? Promise.resolve()
    const next = previous
      .catch(() => {})
      .then(() => forward(helperPath, session, event))
      .catch(error => {
        ctx.logger.warn(`pebrel-deepseek-harness-bridge: event forwarding failed: ${String(error)}`)
      })
      .finally(() => {
        if (queues.get(id) === next) queues.delete(id)
      })
    queues.set(id, next)
  })
}
