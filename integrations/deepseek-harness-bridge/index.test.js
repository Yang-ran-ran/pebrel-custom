import test from 'node:test'
import assert from 'node:assert/strict'

import { apply, toPebrelPayload } from './index.js'

const session = { id: 'session-42', cwd: 'C:/work/project' }

test('maps supported Harness events to Pebrel lifecycle events without forwarding content', () => {
  const cases = [
    ['user/message', 'UserPromptSubmit', {}],
    ['tool/result', 'PostToolUse', { message: { name: 'terminal' } }],
    ['turn/end', 'Stop', {}],
  ]

  for (const [type, hookEvent, data] of cases) {
    const payload = toPebrelPayload(session, {
      type,
      seq: 7,
      time: 1_700_000_000_000,
      data,
    })

    assert.equal(payload.hook_event_name, hookEvent)
    assert.equal(payload.session_id, 'session-42')
    assert.equal(payload.event_id, 'dsh-session-42-7')
    assert.equal(payload.bridge_sequence, 7)
    assert.equal(payload.occurred_at_ms, 1_700_000_000_000)
    assert.equal(payload.cwd, 'C:/work/project')
    assert.equal(payload.source, 'deepseek-harness')
    assert.equal('data' in payload, false)
    assert.equal('message' in payload, false)
    if (type === 'tool/result') assert.equal(payload.tool_name, 'terminal')
  }
})

test('only publishes an absolute workspace cwd for Pane routing', () => {
  const payload = toPebrelPayload(session, {
    type: 'user/message',
    seq: 8,
    time: 1_700_000_000_001,
    data: {},
  })
  assert.equal(payload.cwd, 'C:/work/project')

  const withoutCwd = toPebrelPayload({ id: 'session-43', cwd: 'relative/work' }, {
    type: 'turn/end',
    seq: 9,
    time: 1_700_000_000_002,
    data: {},
  })
  assert.equal('cwd' in withoutCwd, false)
})

test('ignores unsupported and malformed events instead of inventing session identity', () => {
  assert.equal(toPebrelPayload(session, { type: 'turn/start', seq: 1, time: 1 }), undefined)
  assert.equal(toPebrelPayload({}, { type: 'turn/end', seq: 1, time: 1 }), undefined)
  assert.equal(toPebrelPayload({ id: '  ' }, { type: 'turn/end', seq: 1, time: 1 }), undefined)
  assert.equal(toPebrelPayload(session, { type: 'turn/end', seq: NaN, time: 1 }), undefined)
  assert.equal(toPebrelPayload(session, { type: 'turn/end', seq: -1, time: 1 }), undefined)
  assert.equal(toPebrelPayload(session, { type: 'turn/end', seq: 1, time: Infinity }), undefined)
  assert.equal(toPebrelPayload(session, { type: 'turn/end', seq: 1, time: 1.5 }), undefined)
  assert.equal(toPebrelPayload(session, { type: 'turn/end', seq: 1, time: -1 }), undefined)
})

test('does not throw into Cordis for malformed session/event callbacks', () => {
  let listener
  const context = {
    logger: { warn() {} },
    on(name, callback) {
      assert.equal(name, 'session/event')
      listener = callback
    },
  }
  apply(context, { helperPath: '/absolute/pebrel-hook.exe' })
  assert.equal(typeof listener, 'function')
  assert.doesNotThrow(() => listener(null, { type: 'turn/end' }))
  assert.doesNotThrow(() => listener({}, { type: 'turn/end' }))
  assert.doesNotThrow(() => listener(session, null))
  assert.doesNotThrow(() => listener(session, { type: 42 }))
})
