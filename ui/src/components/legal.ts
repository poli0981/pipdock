/**
 * Where PipDock's own documents live, and the five the legal gate lists.
 *
 * One module because the repository URL had been written out three times and was about to be a
 * fourth. More importantly for the document list: `PdLegalGate` records consent against exactly
 * these five, so any other surface offering "the documents you accepted" has to offer the same
 * five or it is describing something the user never saw.
 *
 * URLs are data and are never translated (I18N §2). The labels stay in the catalogs at
 * `legal.documents.*`.
 */

/** The repository itself. Inside `https://github.com/*`, which the opener capability allows. */
export const REPO = 'https://github.com/poli0981/pipdock'

const BLOB = `${REPO}/blob/main`

/** `legal/` holds four; the fifth is the root GPL-3.0 file. */
export const LEGAL_DOCUMENTS = [
  { key: 'license', href: `${BLOB}/LICENSE` },
  { key: 'eula', href: `${BLOB}/legal/EULA.md` },
  { key: 'disclaimer', href: `${BLOB}/legal/DISCLAIMER.md` },
  { key: 'privacy', href: `${BLOB}/legal/PRIVACY-POLICY.md` },
  { key: 'thirdParty', href: `${BLOB}/legal/THIRD-PARTY-NOTICES.md` },
] as const
