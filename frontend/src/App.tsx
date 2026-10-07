import { type FormEvent, useEffect, useRef, useState } from 'react'

type User = {
  id: number
  email: string
  username: string
}

type AuthResponse = {
  user: User
}

async function readError(response: Response) {
  const payload = await response.json().catch(() => null) as { error?: string } | null
  return payload?.error ?? 'Something went wrong. Please try again.'
}

export default function App() {
  const [email, setEmail] = useState('')
  const [user, setUser] = useState<User | null>(null)
  const [sent, setSent] = useState(false)
  const [loading, setLoading] = useState(true)
  const [submitting, setSubmitting] = useState(false)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const authCheckStarted = useRef(false)

  useEffect(() => {
    if (authCheckStarted.current) return
    authCheckStarted.current = true

    const token = new URLSearchParams(window.location.search).get('token')
    if (token) {
      window.history.replaceState({}, '', window.location.pathname)
      void fetch('/api/auth/verify', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        credentials: 'same-origin',
        body: JSON.stringify({ token }),
      })
        .then(async (response) => {
          if (!response.ok) throw new Error(await readError(response))
          const result = await response.json() as AuthResponse
          setUser(result.user)
          setNotice('You are signed in.')
        })
        .catch((reason: unknown) => {
          setError(reason instanceof Error ? reason.message : 'Unable to sign in with that link.')
        })
        .finally(() => setLoading(false))
      return
    }

    void fetch('/api/auth/me', { credentials: 'same-origin' })
      .then(async (response) => {
        if (response.ok) {
          const result = await response.json() as AuthResponse
          setUser(result.user)
        } else if (response.status !== 401) {
          throw new Error(await readError(response))
        }
      })
      .catch((reason: unknown) => {
        setError(reason instanceof Error ? reason.message : 'Unable to check your sign-in.')
      })
      .finally(() => setLoading(false))
  }, [])

  async function requestLink(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setSubmitting(true)
    setError('')
    setNotice('')
    try {
      const response = await fetch('/api/auth/request-link', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        credentials: 'same-origin',
        body: JSON.stringify({ email }),
      })
      if (!response.ok) throw new Error(await readError(response))
      setSent(true)
      setNotice('Check your email for a sign-in link. The link expires in 15 minutes.')
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Unable to request a sign-in link.')
    } finally {
      setSubmitting(false)
    }
  }

  async function logout() {
    setSubmitting(true)
    setError('')
    try {
      const response = await fetch('/api/auth/logout', {
        method: 'POST',
        credentials: 'same-origin',
      })
      if (!response.ok) throw new Error(await readError(response))
      setUser(null)
      setSent(false)
      setNotice('You have been signed out.')
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Unable to sign out.')
    } finally {
      setSubmitting(false)
    }
  }

  return (
    <main className="shell">
      <section className="card" aria-labelledby="page-title">
        <p className="eyebrow">Gossip Board</p>
        {user ? (
          <div className="auth-content">
            <h1 id="page-title">You’re in.</h1>
            <p className="subtitle">Signed in as <strong>{user.email}</strong></p>
            <button className="secondary-button" onClick={logout} disabled={submitting}>
              {submitting ? 'Signing out…' : 'Sign out'}
            </button>
          </div>
        ) : (
          <div className="auth-content">
            <h1 id="page-title">Share the latest whispers.</h1>
            <p className="subtitle">
              Sign in with your email and we’ll send you a secure, one-time link.
              In local development, find it in the backend console.
            </p>
            <form className="login-form" onSubmit={requestLink}>
              <label htmlFor="email">Email address</label>
              <input
                id="email"
                type="email"
                autoComplete="email"
                required
                maxLength={320}
                value={email}
                onChange={(event) => setEmail(event.target.value)}
                placeholder="you@example.com"
                disabled={submitting || sent}
              />
              <button type="submit" disabled={submitting || sent}>
                {submitting ? 'Sending link…' : sent ? 'Link sent' : 'Email me a sign-in link'}
              </button>
              {sent && (
                <button
                  className="text-button"
                  type="button"
                  onClick={() => {
                    setSent(false)
                    setNotice('')
                  }}
                >
                  Use a different email
                </button>
              )}
            </form>
          </div>
        )}
        {loading && <p className="feedback" role="status">Checking your sign-in…</p>}
        {notice && !loading && <p className="feedback success" role="status">{notice}</p>}
        {error && <p className="feedback error" role="alert">{error}</p>}
      </section>
    </main>
  )
}
