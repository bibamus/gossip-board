import { type FormEvent, useEffect, useRef, useState } from 'react'
import { readError } from './api'
import PostInteractions from './PostInteractions'

type User = {
  id: number
  email: string
  username: string
}

type AuthResponse = {
  user: User
}

type Post = {
  id: number
  author_id: number
  title: string
  body: string
  created_at: string
  updated_at: string
}

type ShareRecipient = {
  id: number
  post_id: number
  shared_by_user_id: number
  shared_with_user_id: number
  username: string
  created_at: string
}

type UserSuggestion = {
  id: number
  username: string
}

type PostDraft = {
  title: string
  body: string
}

function formatDate(value: string) {
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: 'medium',
    timeStyle: 'short',
  }).format(new Date(value))
}

export default function App() {
  const [email, setEmail] = useState('')
  const [user, setUser] = useState<User | null>(null)
  const [sent, setSent] = useState(false)
  const [loading, setLoading] = useState(true)
  const [submitting, setSubmitting] = useState(false)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const [posts, setPosts] = useState<Post[]>([])
  const [postsLoading, setPostsLoading] = useState(false)
  const [feedRefresh, setFeedRefresh] = useState(0)
  const [selectedPost, setSelectedPost] = useState<Post | null>(null)
  const [shares, setShares] = useState<ShareRecipient[]>([])
  const [sharesLoading, setSharesLoading] = useState(false)
  const [shareUsername, setShareUsername] = useState('')
  const [userSuggestions, setUserSuggestions] = useState<UserSuggestion[]>([])
  const [suggestionsLoading, setSuggestionsLoading] = useState(false)
  const [suggestionError, setSuggestionError] = useState('')
  const [editingPost, setEditingPost] = useState<Post | null | undefined>(undefined)
  const [draft, setDraft] = useState<PostDraft>({ title: '', body: '' })
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

  useEffect(() => {
    if (!user) {
      setPosts([])
      setSelectedPost(null)
      return
    }

    let cancelled = false
    setPostsLoading(true)
    void fetch('/api/posts', { credentials: 'same-origin' })
      .then(async (response) => {
        if (!response.ok) throw new Error(await readError(response))
        return response.json() as Promise<Post[]>
      })
      .then((result) => {
        if (!cancelled) setPosts(result)
      })
      .catch((reason: unknown) => {
        if (!cancelled) {
          setError(reason instanceof Error ? reason.message : 'Unable to load posts.')
        }
      })
      .finally(() => {
        if (!cancelled) setPostsLoading(false)
      })

    return () => {
      cancelled = true
    }
  }, [user, feedRefresh])

  useEffect(() => {
    if (!selectedPost || !user) {
      setShares([])
      return
    }

    let cancelled = false
    setSharesLoading(true)
    void fetch(`/api/posts/${selectedPost.id}/shares`, { credentials: 'same-origin' })
      .then(async (response) => {
        if (!response.ok) throw new Error(await readError(response))
        return response.json() as Promise<ShareRecipient[]>
      })
      .then((result) => {
        if (!cancelled) setShares(result)
      })
      .catch((reason: unknown) => {
        if (!cancelled) {
          setError(reason instanceof Error ? reason.message : 'Unable to load post shares.')
        }
      })
      .finally(() => {
        if (!cancelled) setSharesLoading(false)
      })

    return () => {
      cancelled = true
    }
  }, [selectedPost, user])

  useEffect(() => {
    const query = shareUsername.trim()
    setUserSuggestions([])
    setSuggestionsLoading(false)
    if (!selectedPost || !user || query.length < 2) {
      setSuggestionError('')
      return
    }

    const controller = new AbortController()
    const timeout = window.setTimeout(() => {
      setSuggestionsLoading(true)
      setSuggestionError('')
      const params = new URLSearchParams({ q: query })
      void fetch(`/api/users/search?${params}`, {
        credentials: 'same-origin',
        signal: controller.signal,
      })
        .then(async (response) => {
          if (!response.ok) throw new Error(await readError(response))
          return response.json() as Promise<UserSuggestion[]>
        })
        .then(setUserSuggestions)
        .catch((reason: unknown) => {
          if (!controller.signal.aborted) {
            setSuggestionError(reason instanceof Error ? reason.message : 'Unable to search for people.')
          }
        })
        .finally(() => {
          if (!controller.signal.aborted) setSuggestionsLoading(false)
        })
    }, 200)

    return () => {
      window.clearTimeout(timeout)
      controller.abort()
    }
  }, [shareUsername, selectedPost, user])

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
      setEditingPost(undefined)
      setNotice('You have been signed out.')
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Unable to sign out.')
    } finally {
      setSubmitting(false)
    }
  }

  function startCreate() {
    setSelectedPost(null)
    setEditingPost(null)
    setDraft({ title: '', body: '' })
    setError('')
    setNotice('')
  }

  function startEdit(post: Post) {
    setSelectedPost(post)
    setEditingPost(post)
    setDraft({ title: post.title, body: post.body })
    setError('')
    setNotice('')
  }

  function closeEditor() {
    setEditingPost(undefined)
    setError('')
  }

  async function savePost(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!user) return
    setSubmitting(true)
    setError('')
    setNotice('')
    const isEditing = editingPost !== null && editingPost !== undefined
    const endpoint = isEditing ? `/api/posts/${editingPost.id}` : '/api/posts'
    try {
      const response = await fetch(endpoint, {
        method: isEditing ? 'PUT' : 'POST',
        headers: { 'Content-Type': 'application/json' },
        credentials: 'same-origin',
        body: JSON.stringify(draft),
      })
      if (!response.ok) throw new Error(await readError(response))
      const savedPost = await response.json() as Post
      setPosts((current) => isEditing
        ? current.map((post) => post.id === savedPost.id ? savedPost : post)
        : [savedPost, ...current])
      setSelectedPost(savedPost)
      setEditingPost(undefined)
      setNotice(isEditing ? 'Post updated.' : 'Post published.')
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Unable to save the post.')
    } finally {
      setSubmitting(false)
    }
  }

  async function deletePost(post: Post) {
    if (!window.confirm('Delete this post? This cannot be undone.')) return
    setSubmitting(true)
    setError('')
    setNotice('')
    try {
      const response = await fetch(`/api/posts/${post.id}`, {
        method: 'DELETE',
        credentials: 'same-origin',
      })
      if (!response.ok) throw new Error(await readError(response))
      setPosts((current) => current.filter((item) => item.id !== post.id))
      setSelectedPost(null)
      setEditingPost(undefined)
      setNotice('Post deleted.')
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Unable to delete the post.')
    } finally {
      setSubmitting(false)
    }
  }

  async function sharePost(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!selectedPost) return
    setSubmitting(true)
    setError('')
    setNotice('')
    try {
      const response = await fetch(`/api/posts/${selectedPost.id}/shares`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        credentials: 'same-origin',
        body: JSON.stringify({ username: shareUsername }),
      })
      if (!response.ok) throw new Error(await readError(response))
      const recipient = await response.json() as ShareRecipient
      setShares((current) => [...current, recipient])
      setShareUsername('')
      setNotice(`Shared with @${recipient.username}.`)
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Unable to share this post.')
    } finally {
      setSubmitting(false)
    }
  }

  const isEditing = editingPost !== undefined

  return (
    <main className={user ? 'app-shell' : 'shell'}>
      {!user ? (
        <section className="card" aria-labelledby="page-title">
          <p className="eyebrow">Gossip Board</p>
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
          {loading && <p className="feedback" role="status">Checking your sign-in…</p>}
          {notice && !loading && <p className="feedback success" role="status">{notice}</p>}
          {error && <p className="feedback error" role="alert">{error}</p>}
        </section>
      ) : (
        <div className="dashboard">
          <header className="dashboard-header">
            <a className="brand" href="/" onClick={(event) => {
              event.preventDefault()
              setSelectedPost(null)
              setEditingPost(undefined)
              setNotice('')
            }}>Gossip Board</a>
            <div className="account">
              <span>{user.email}</span>
              <button className="quiet-button" onClick={logout} disabled={submitting}>Sign out</button>
            </div>
          </header>

          <section className="dashboard-content" aria-labelledby="page-title">
            <div className="dashboard-heading">
              <div>
                <p className="eyebrow">Your board</p>
                <h1 id="page-title">
                  {selectedPost
                    ? 'The story'
                    : isEditing
                      ? editingPost ? 'Edit your post' : 'Write a post'
                      : 'Latest whispers.'}
                </h1>
              </div>
              {!selectedPost && !isEditing && (
                <div className="form-actions dashboard-actions">
                  <button
                    className="quiet-button"
                    onClick={() => setFeedRefresh((current) => current + 1)}
                    disabled={postsLoading}
                  >
                    {postsLoading ? 'Refreshing…' : 'Refresh feed'}
                  </button>
                  <button className="primary-button" onClick={startCreate}>Write a post</button>
                </div>
              )}
            </div>

            {isEditing ? (
              <form className="post-form" onSubmit={savePost}>
                <label htmlFor="post-title">Title</label>
                <input
                  id="post-title"
                  required
                  maxLength={200}
                  value={draft.title}
                  onChange={(event) => setDraft({ ...draft, title: event.target.value })}
                  placeholder="What’s the story?"
                  disabled={submitting}
                />
                <label htmlFor="post-body">Gossip</label>
                <textarea
                  id="post-body"
                  required
                  rows={9}
                  value={draft.body}
                  onChange={(event) => setDraft({ ...draft, body: event.target.value })}
                  placeholder="Tell us what happened…"
                  disabled={submitting}
                />
                <div className="form-actions">
                  <button
                    className="primary-button"
                    type="submit"
                    disabled={submitting}
                  >
                    {submitting ? 'Saving…' : editingPost ? 'Save changes' : 'Publish post'}
                  </button>
                  <button className="quiet-button" type="button" onClick={closeEditor} disabled={submitting}>
                    Cancel
                  </button>
                </div>
              </form>
            ) : selectedPost ? (
              <article className="post-detail">
                <button className="back-button" onClick={() => {
                  setSelectedPost(null)
                  setNotice('')
                }}>← All posts</button>
                <p className="post-meta">
                  {selectedPost.author_id === user.id ? 'Posted by you' : 'Shared with you'}
                  {' · '}{formatDate(selectedPost.created_at)}
                  {selectedPost.updated_at !== selectedPost.created_at && ' · Edited'}
                </p>
                <h2>{selectedPost.title}</h2>
                <p className="post-body">{selectedPost.body}</p>
                <PostInteractions key={selectedPost.id} postId={selectedPost.id} />
                {selectedPost.author_id === user.id && (
                  <div className="form-actions detail-actions">
                    <button className="secondary-button" onClick={() => startEdit(selectedPost)} disabled={submitting}>
                      Edit post
                    </button>
                    <button className="danger-button" onClick={() => void deletePost(selectedPost)} disabled={submitting}>
                      Delete post
                    </button>
                  </div>
                )}
                <section className="sharing-section" aria-labelledby="sharing-title">
                  <h3 id="sharing-title">Share this post</h3>
                  <p>Share with someone who already has a Gossip Board account. Anyone with access can share it onward.</p>
                  <form className="share-form" onSubmit={sharePost}>
                    <label className="visually-hidden" htmlFor="share-username">
                      Recipient username
                    </label>
                    <input
                      id="share-username"
                      type="text"
                      autoComplete="off"
                      required
                      maxLength={50}
                      list="recipient-suggestions"
                      value={shareUsername}
                      onChange={(event) => setShareUsername(event.target.value)}
                      placeholder="@username"
                      disabled={submitting}
                    />
                    <datalist id="recipient-suggestions">
                      {userSuggestions.map((suggestion) => (
                        <option key={suggestion.id} value={suggestion.username} />
                      ))}
                    </datalist>
                    <button
                      className="primary-button"
                      type="submit"
                      disabled={submitting || sharesLoading}
                    >
                      {submitting ? 'Sharing…' : 'Share post'}
                    </button>
                  </form>
                  {suggestionsLoading && (
                    <p className="post-meta" role="status">Searching usernames…</p>
                  )}
                  {suggestionError && <p className="feedback error" role="alert">{suggestionError}</p>}
                  <div className="share-list">
                    <h4>Shared with</h4>
                    {sharesLoading ? (
                      <p className="post-meta" role="status">Loading recipients…</p>
                    ) : shares.length > 0 ? (
                      <ul>
                        {shares.map((recipient) => (
                          <li key={recipient.id}>
                            <span>@{recipient.username}</span>
                            <span className="post-meta">
                              {recipient.shared_by_user_id === user.id
                                ? 'Shared by you'
                                : recipient.shared_by_user_id === selectedPost.author_id
                                  ? 'Shared by the author'
                                  : 'Shared onward'}
                            </span>
                          </li>
                        ))}
                      </ul>
                    ) : (
                      <p className="post-meta">No one else has access yet.</p>
                    )}
                  </div>
                </section>
              </article>
            ) : postsLoading ? (
              <p className="empty-state" role="status">Loading your posts…</p>
            ) : posts.length === 0 ? (
              <div className="empty-state">
                <h2>Your board is quiet.</h2>
                <p>Be the first to share a story.</p>
                <button className="primary-button" onClick={startCreate}>Write your first post</button>
              </div>
            ) : (
              <div className="post-list">
                {posts.map((post) => (
                  <button className="post-card" key={post.id} onClick={() => {
                    setSelectedPost(post)
                    setNotice('')
                  }}>
                    <span className="post-meta">
                      {post.author_id === user.id ? 'Your post' : 'Shared with you'}
                      {' · '}{formatDate(post.created_at)}
                    </span>
                    <strong>{post.title}</strong>
                    <span className="post-preview">{post.body}</span>
                  </button>
                ))}
              </div>
            )}
            {notice && !loading && <p className="feedback success" role="status">{notice}</p>}
            {error && <p className="feedback error" role="alert">{error}</p>}
          </section>
        </div>
      )}
    </main>
  )
}
