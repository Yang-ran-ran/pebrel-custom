import test from 'node:test'
import assert from 'node:assert/strict'
import { toPebrelPayload } from '../index.js'

test('maps supported Harness session events into Pebrel lifecycle events', () => {
  const session = { id: 'session-123', cwd: 'C:/work/project' }
  const cases = [
    ['user/message', 'UserPromptSubmit'],
    ['tool/result', 'PostToolUse'],
    ['turn/end', 'Stop'],
  ]
  for (const [type, hook_event_name] of cases) {
    const payload = toPebrelPayload(session, { type, seq: 9, time: 1234, data: { name: 'read_file' } })
    assert.equal(payload.hook_event_name, hook_event_name)
    assert.equal(payload.session_id, 'session-123')
    assert.equal(payload.cwd, 'C:/work/project')
    assert.equal(payload.bridge_sequence, 9)
  }
})

test('does not forward message bodies, tool arguments, or unknown events', () => {
  const session = { id: 's', cwd: 'C:/work' }
  const prompt = toPebrelPayload(session, {
    type: 'user/message', seq: 1, time: 1,
    data: { content: 'private prompt text', source: { kind: 'user' } },
  })
  assert.equal(JSON.stringify(prompt).includes('private prompt text'), false)
  const tool = toPebrelPayload(session, {
    type: 'tool/result', seq: 2, time: 2,
    data: { message: { name: 'shell', content: [{ type: 'text', text: 'private result' }] } },
  })
  assert.equal(JSON.stringify(tool).includes('private result'), false)
  assert.equal(toPebrelPayload(session, { type: 'session/disposed', seq: 3, time: 3, data: {} }), undefined)
})
