/**
 * Maintenance-mode triage — run by .github/workflows/triage.yml through actions/github-script.
 *
 * Since 1.6.0 PipDock takes bug fixes, UI/UX and accessibility problems, wording and translation
 * fixes, and dependency and security updates, and no new features (CONTRIBUTING.md, "Scope in
 * maintenance mode"). A feature request is labelled `enhancement` and `out-of-scope`, answered in
 * English and Vietnamese, and closed as not planned.
 *
 * **Only on signals that cannot be a bug report**, because closing a real report would be worse
 * than leaving a request open: a title that says it is a feature (`[Feature]`, `Feature request:`,
 * `[Tính năng]` …), a pull request titled with the Conventional Commits `feat` type the repository
 * uses, or the `enhancement` label being put on. Words in a body decide nothing.
 *
 * **Never** for the owner, members and collaborators, for bots (Dependabot), or for anything
 * labelled `policy-exception` — the maintainer's way of keeping open a feature judged truly
 * necessary, which the policy allows.
 *
 * `classify` is pure and tested with `node --test` (triage.test.cjs); `run` is the thin part that
 * talks to GitHub.
 */

'use strict'

const MAINTAINERS = new Set(['OWNER', 'MEMBER', 'COLLABORATOR'])
const EXEMPT = 'policy-exception'
const FEATURE_LABEL = 'enhancement'
const OUT_OF_SCOPE = 'out-of-scope'
const POLICY_URL =
  'https://github.com/poli0981org/pipdock/blob/main/CONTRIBUTING.md#scope-in-maintenance-mode'
const IDEAS_URL = 'https://github.com/poli0981org/pipdock/discussions/categories/ideas'

/** `[Feature] …`, `[Feature request]: …`, `Feature request: …`, `[Tính năng] …`, `Đề xuất tính năng: …`. */
const FEATURE_TITLE =
  /^\s*(?:\[\s*(?:feature(?:\s+request)?|enhancement|request|tính năng|đề xuất(?:\s+tính năng)?)\s*\]|(?:feature\s+request|feature|enhancement|đề xuất tính năng|tính năng mới)\s*:)/iu

/** `feat: …`, `feat(ui): …`, `feat!: …` — the Conventional Commits type this repository uses. */
const FEAT_COMMIT = /^\s*feat(?:\([^)]*\))?!?:/i

/**
 * Whether an issue or pull request is a feature request this policy closes.
 *
 * @param {object} item
 * @param {'issue'|'pull_request'} item.kind
 * @param {string} item.title
 * @param {string[]} item.labels Names of the labels it carries now.
 * @param {string} item.association The author's `author_association`.
 * @param {boolean} item.authorIsBot
 * @param {string} item.state `open` or `closed`.
 * @param {string|null} item.addedLabel The label a `labeled` event put on, else null.
 * @returns {{close: boolean, reason: string}}
 */
function classify(item) {
  if (item.state !== 'open') return { close: false, reason: 'not open' }
  if (item.authorIsBot) return { close: false, reason: 'opened by a bot' }
  if (MAINTAINERS.has(item.association)) return { close: false, reason: 'opened by a maintainer' }
  if (item.labels.includes(EXEMPT)) return { close: false, reason: `labelled ${EXEMPT}` }
  if (item.labels.includes(OUT_OF_SCOPE)) return { close: false, reason: 'already handled' }

  if (item.addedLabel === FEATURE_LABEL) return { close: true, reason: `labelled ${FEATURE_LABEL}` }
  if (item.addedLabel !== null) return { close: false, reason: `labelled ${item.addedLabel}` }
  if (item.kind === 'pull_request' && FEAT_COMMIT.test(item.title)) {
    return { close: true, reason: 'a feat: pull request' }
  }
  if (FEATURE_TITLE.test(item.title)) return { close: true, reason: 'titled as a feature request' }
  return { close: false, reason: 'not a feature request' }
}

/** The answer left on a closed request, in both of the project's languages. */
function comment(kind) {
  const what = kind === 'pull_request' ? 'pull request' : 'issue'
  const whatVi = kind === 'pull_request' ? 'Pull request' : 'Issue'
  return [
    'Thanks for taking the time to write this.',
    '',
    `PipDock has been in **maintenance mode** since 1.6.0: it takes bug fixes, UI/UX and accessibility problems, wording and translation fixes, and dependency and security updates — and no new features. This ${what} reads as a feature request, so it has been labelled \`${OUT_OF_SCOPE}\` and closed. [CONTRIBUTING.md](${POLICY_URL}) says what is accepted and why.`,
    '',
    `If it is really a bug or a barrier, say so in a comment and it will be looked at again. If you believe the feature is truly necessary, make the case in [Discussions → Ideas](${IDEAS_URL}).`,
    '',
    '---',
    '',
    `Cảm ơn bạn đã dành thời gian viết. Từ bản 1.6.0, PipDock ở **giai đoạn bảo trì**: chỉ nhận sửa lỗi, lỗi UI/UX và khả năng tiếp cận, sửa câu chữ và bản dịch, cập nhật phụ thuộc và bảo mật — không nhận tính năng mới. ${whatVi} này là một đề xuất tính năng, nên đã được gắn nhãn \`${OUT_OF_SCOPE}\` và đóng lại. Nếu đây thật ra là lỗi, hãy bình luận để được xem lại; nếu bạn tin tính năng này thật sự cần thiết, hãy trình bày ở [Discussions → Ideas](${IDEAS_URL}).`,
  ].join('\n')
}

/** The item an `issues` or `pull_request_target` event is about, in `classify`'s shape. */
function itemOf(payload) {
  const kind = payload.pull_request ? 'pull_request' : 'issue'
  const raw = payload.pull_request ?? payload.issue
  return {
    kind,
    number: raw.number,
    title: raw.title ?? '',
    labels: (raw.labels ?? []).map((l) => l.name),
    association: raw.author_association ?? 'NONE',
    authorIsBot: raw.user?.type === 'Bot',
    state: raw.state,
    addedLabel: payload.action === 'labeled' ? (payload.label?.name ?? null) : null,
  }
}

/** Entry point for actions/github-script. */
async function run({ github, context, core }) {
  if (context.eventName === 'workflow_dispatch') {
    // A dry run: classify what the inputs describe and say so. Nothing is touched.
    const inputs = context.payload.inputs ?? {}
    const item = {
      kind: inputs.kind === 'pull_request' ? 'pull_request' : 'issue',
      title: inputs.title ?? '',
      labels: (inputs.labels ?? '').split(',').map((s) => s.trim()).filter(Boolean),
      association: inputs.association ?? 'NONE',
      authorIsBot: false,
      state: 'open',
      addedLabel: null,
    }
    const verdict = classify(item)
    core.notice(`${verdict.close ? 'would close' : 'would leave open'}: ${verdict.reason}`)
    core.summary.addRaw(`**${verdict.close ? 'Would close' : 'Would leave open'}** — ${verdict.reason}`)
    await core.summary.write()
    return verdict
  }

  const item = itemOf(context.payload)
  const verdict = classify(item)
  core.info(`#${item.number}: ${verdict.close ? 'closing' : 'leaving open'} — ${verdict.reason}`)
  if (!verdict.close) return verdict

  const { owner, repo } = context.repo
  await github.rest.issues.addLabels({
    owner,
    repo,
    issue_number: item.number,
    labels: [FEATURE_LABEL, OUT_OF_SCOPE],
  })
  await github.rest.issues.createComment({
    owner,
    repo,
    issue_number: item.number,
    body: comment(item.kind),
  })
  if (item.kind === 'pull_request') {
    await github.rest.pulls.update({ owner, repo, pull_number: item.number, state: 'closed' })
  } else {
    await github.rest.issues.update({
      owner,
      repo,
      issue_number: item.number,
      state: 'closed',
      state_reason: 'not_planned',
    })
  }
  return verdict
}

module.exports = { classify, comment, itemOf, run, FEATURE_TITLE, FEAT_COMMIT }
