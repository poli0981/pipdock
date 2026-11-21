// `node --test .github/scripts/*.test.cjs` — Node's built-in runner, as for scripts/*.test.mjs.

'use strict'

const assert = require('node:assert/strict')
const { test } = require('node:test')

const { classify, comment, itemOf } = require('./triage.cjs')

const base = {
  kind: 'issue',
  title: '',
  labels: [],
  association: 'NONE',
  authorIsBot: false,
  state: 'open',
  addedLabel: null,
}
const item = (over) => ({ ...base, ...over })

test('a title that says it is a feature request is closed, in either language', () => {
  for (const title of [
    '[Feature] Dark/light theme toggle',
    '[Feature request]: conda support',
    '[Enhancement] faster search',
    '[Request] macOS build',
    'Feature request: export to Poetry',
    'feature: portable mode',
    '[Tính năng] Hỗ trợ conda',
    '[Đề xuất tính năng] chế độ portable',
    'Đề xuất tính năng: giao diện sáng',
  ]) {
    assert.equal(classify(item({ title })).close, true, title)
  }
})

test('a bug report is never closed, whatever words it uses', () => {
  for (const title of [
    '[Bug]: crash when adding an interpreter',
    'The feature to roll back is broken',
    'Rollback feature does nothing',
    'Request timed out while searching',
    'Enhancement of the dock bay stopped working after 1.6.0',
    '[Accessibility]: Remove from list has no focus ring',
  ]) {
    assert.equal(classify(item({ title })).close, false, title)
  }
})

test('a pull request titled feat: is closed; fix:, docs: and build(deps): are not', () => {
  for (const title of ['feat: conda envs', 'feat(ui): light theme', 'feat!: drop pip']) {
    assert.equal(classify(item({ kind: 'pull_request', title })).close, true, title)
  }
  for (const title of ['fix(ui): focus ring', 'docs: typo', 'build(deps): Bump vite', 'i18n(vi): sửa lỗi chính tả']) {
    assert.equal(classify(item({ kind: 'pull_request', title })).close, false, title)
  }
  // An issue titled like a commit is a person's wording, not a commit type.
  assert.equal(classify(item({ kind: 'issue', title: 'feat: something' })).close, false)
})

test('putting the enhancement label on closes it; any other label does not', () => {
  assert.equal(classify(item({ title: 'Make it faster', addedLabel: 'enhancement' })).close, true)
  // A label event is about the label: a feature title already open and now labelled `bug` was
  // let through when it was opened, and re-judging it on every label would close it late.
  assert.equal(classify(item({ title: '[Feature] x', addedLabel: 'bug' })).close, false)
})

test('maintainers, bots and policy exceptions are never closed', () => {
  for (const association of ['OWNER', 'MEMBER', 'COLLABORATOR']) {
    assert.equal(classify(item({ title: '[Feature] x', association })).close, false, association)
  }
  assert.equal(classify(item({ kind: 'pull_request', title: 'feat: x', authorIsBot: true })).close, false)
  assert.equal(
    classify(item({ title: '[Feature] x', labels: ['policy-exception'], addedLabel: 'enhancement' })).close,
    false,
  )
})

test('what is closed or already handled is left alone', () => {
  assert.equal(classify(item({ title: '[Feature] x', state: 'closed' })).close, false)
  assert.equal(classify(item({ title: '[Feature] x', labels: ['out-of-scope'] })).close, false)
})

test('the answer is in both languages and says what kind of item it is', () => {
  const pr = comment('pull_request')
  assert.match(pr, /This pull request reads as a feature request/)
  assert.match(pr, /Pull request này/)
  assert.match(pr, /CONTRIBUTING\.md#scope-in-maintenance-mode/)
  assert.match(comment('issue'), /This issue reads as a feature request/)
})

test('an event payload becomes an item, the added label included', () => {
  const fromIssue = itemOf({
    action: 'labeled',
    label: { name: 'enhancement' },
    issue: {
      number: 7,
      title: 'Make it faster',
      labels: [{ name: 'triage' }, { name: 'enhancement' }],
      author_association: 'NONE',
      user: { type: 'User' },
      state: 'open',
    },
  })
  assert.deepEqual(fromIssue, {
    kind: 'issue',
    number: 7,
    title: 'Make it faster',
    labels: ['triage', 'enhancement'],
    association: 'NONE',
    authorIsBot: false,
    state: 'open',
    addedLabel: 'enhancement',
  })
  const fromPr = itemOf({
    action: 'opened',
    pull_request: {
      number: 9,
      title: 'feat: x',
      labels: [],
      author_association: 'FIRST_TIME_CONTRIBUTOR',
      user: { type: 'User' },
      state: 'open',
    },
  })
  assert.equal(fromPr.kind, 'pull_request')
  assert.equal(fromPr.addedLabel, null)
  assert.equal(classify(fromPr).close, true)
})
