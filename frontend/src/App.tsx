import {
  type ClipboardEvent as ReactClipboardEvent,
  type DragEvent,
  type FormEvent,
  useEffect,
  useRef,
  useState,
} from 'react'
import { readError } from './api'
import PostInteractions from './PostInteractions'
import TagPicker, { type Tag } from './TagPicker'

type User = {
  id: number
  email: string
  username: string
  is_admin: boolean
}

type AuthResponse = {
  user: User
}

type AdminOverview = {
  users: {
    id: number
    email: string
    username: string
    created_at: string
  }[]
  post_count: number
}

type Post = {
  id: number
  author_id: number
  title: string
  body: string
  image_data: string | null
  created_at: string
  updated_at: string
  tags: Tag[]
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
  tags: Tag[]
  imageData: string
}

const MAX_IMAGE_BYTES = 3 * 1024 * 1024
const IMAGE_TYPES = ['image/png', 'image/jpeg', 'image/gif', 'image/webp']

function formatDate(value: string) {
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: 'medium',
    timeStyle: 'short',
  }).format(new Date(value))
}

function readImageFile(file: File) {
  if (!IMAGE_TYPES.includes(file.type)) {
    return Promise.reject(new Error('Choose a PNG, JPEG, GIF, or WebP image.'))
  }
  if (file.size > MAX_IMAGE_BYTES) {
    return Promise.reject(new Error('Images must be 3 MB or smaller.'))
  }
  return new Promise<string>((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => {
      if (typeof reader.result !== 'string') {
        reject(new Error('Unable to read this image.'))
        return
      }
      resolve(reader.result)
    }
    reader.onerror = () => reject(new Error('Unable to read this image.'))
    reader.readAsDataURL(file)
  })
}

function pastedImage(data: DataTransfer) {
  for (const item of Array.from(data.items)) {
    if (item.kind === 'file' && item.type.startsWith('image/')) {
      return item.getAsFile()
    }
  }
  return Array.from(data.files).find((file) => file.type.startsWith('image/')) ?? null
}

export default function App() {
  const [email, setEmail] = useState('')
  const [user, setUser] = useState<User | null>(null)
  const [sent, setSent] = useState(false)
  const [loading, setLoading] = useState(true)
  const [submitting, setSubmitting] = useState(false)
  const [imageLoading, setImageLoading] = useState(false)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const [posts, setPosts] = useState<Post[]>([])
  const [postsLoading, setPostsLoading] = useState(false)
  const [feedRefresh, setFeedRefresh] = useState(0)
  const [topics, setTopics] = useState<Tag[]>([])
  const [topicFilter, setTopicFilter] = useState('')
  const [topicsLoading, setTopicsLoading] = useState(false)
  const [tagsSaving, setTagsSaving] = useState(false)
  const [selectedPost, setSelectedPost] = useState<Post | null>(null)
  const [shares, setShares] = useState<ShareRecipient[]>([])
  const [sharesLoading, setSharesLoading] = useState(false)
  const [sharesLoadedPostId, setSharesLoadedPostId] = useState<number | null>(null)
  const [shareUsername, setShareUsername] = useState('')
  const [userSuggestions, setUserSuggestions] = useState<UserSuggestion[]>([])
  const [suggestionsLoading, setSuggestionsLoading] = useState(false)
  const [suggestionError, setSuggestionError] = useState('')
  const [editingPost, setEditingPost] = useState<Post | null | undefined>(undefined)
  const [draft, setDraft] = useState<PostDraft>({ title: '', body: '', tags: [], imageData: '' })
  const [settingsOpen, setSettingsOpen] = useState(false)
  const [adminOpen, setAdminOpen] = useState(false)
  const [adminOverview, setAdminOverview] = useState<AdminOverview | null>(null)
  const [adminLoading, setAdminLoading] = useState(false)
  const [adminRefresh, setAdminRefresh] = useState(0)
  const [usernameDraft, setUsernameDraft] = useState('')
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
    const params = new URLSearchParams()
    if (topicFilter) params.set('tag', topicFilter)
    void fetch(`/api/posts?${params}`, { credentials: 'same-origin' })
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
  }, [user, feedRefresh, topicFilter])

  useEffect(() => {
    if (!adminOpen || !user?.is_admin) {
      setAdminOverview(null)
      return
    }

    let cancelled = false
    setAdminLoading(true)
    void fetch('/api/admin/overview', { credentials: 'same-origin' })
      .then(async (response) => {
        if (!response.ok) throw new Error(await readError(response))
        return response.json() as Promise<AdminOverview>
      })
      .then((result) => {
        if (!cancelled) setAdminOverview(result)
      })
      .catch((reason: unknown) => {
        if (!cancelled) {
          setError(reason instanceof Error ? reason.message : 'Unable to load the admin overview.')
        }
      })
      .finally(() => {
        if (!cancelled) setAdminLoading(false)
      })

    return () => {
      cancelled = true
    }
  }, [adminOpen, adminRefresh, user])

  useEffect(() => {
    if (!user) {
      setTopics([])
      setTopicFilter('')
      return
    }
    let cancelled = false
    setTopicsLoading(true)
    void fetch('/api/tags', { credentials: 'same-origin' })
      .then(async (response) => {
        if (!response.ok) throw new Error(await readError(response))
        return response.json() as Promise<Tag[]>
      })
      .then((result) => {
        if (!cancelled) setTopics(result)
      })
      .catch((reason: unknown) => {
        if (!cancelled) {
          setError(reason instanceof Error ? reason.message : 'Unable to load topics.')
        }
      })
      .finally(() => {
        if (!cancelled) setTopicsLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [user, feedRefresh])

  useEffect(() => {
    setSharesLoadedPostId(null)
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
        if (!cancelled) {
          setShares(result)
          setSharesLoadedPostId(selectedPost.id)
        }
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
      setSettingsOpen(false)
      setAdminOpen(false)
      setNotice('You have been signed out.')
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Unable to sign out.')
    } finally {
      setSubmitting(false)
    }
  }

  function toggleSettings() {
    if (!settingsOpen && user) setUsernameDraft(user.username)
    setSettingsOpen((current) => !current)
    setAdminOpen(false)
    setError('')
    setNotice('')
    setSelectedPost(null)
    setEditingPost(undefined)
  }

  async function updateUsername(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setSubmitting(true)
    setError('')
    setNotice('')
    try {
      const response = await fetch('/api/auth/me', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        credentials: 'same-origin',
        body: JSON.stringify({ username: usernameDraft }),
      })
      if (!response.ok) throw new Error(await readError(response))
      const result = await response.json() as AuthResponse
      setUser(result.user)
      setUsernameDraft(result.user.username)
      setNotice('Username updated.')
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Unable to update your username.')
    } finally {
      setSubmitting(false)
    }
  }

  async function deleteAccount() {
    if (!window.confirm(
      'Permanently delete your account? Your posts, comments, votes, and shares will also be deleted. This cannot be undone.',
    )) return

    setSubmitting(true)
    setError('')
    setNotice('')
    try {
      const response = await fetch('/api/auth/me', {
        method: 'DELETE',
        credentials: 'same-origin',
      })
      if (!response.ok) throw new Error(await readError(response))
      setUser(null)
      setSettingsOpen(false)
      setAdminOpen(false)
      setSent(false)
      setNotice('')
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Unable to delete your account.')
    } finally {
      setSubmitting(false)
    }
  }

  function startCreate() {
    setSelectedPost(null)
    setEditingPost(null)
    setDraft({ title: '', body: '', tags: [], imageData: '' })
    setError('')
    setNotice('')
  }

  function startEdit(post: Post) {
    setSelectedPost(post)
    setEditingPost(post)
    setDraft({
      title: post.title,
      body: post.body,
      tags: post.tags,
      imageData: post.image_data ?? '',
    })
    setError('')
    setNotice('')
  }

  function closeEditor() {
    setEditingPost(undefined)
    setError('')
  }

  async function attachImage(file: File) {
    setError('')
    setImageLoading(true)
    try {
      const imageData = await readImageFile(file)
      setDraft((current) => ({ ...current, imageData }))
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Unable to read this image.')
    } finally {
      setImageLoading(false)
    }
  }

  function handleImageDrop(event: DragEvent<HTMLDivElement>) {
    event.preventDefault()
    const files = Array.from(event.dataTransfer.files)
    const file = files.find((candidate) => candidate.type.startsWith('image/'))
    if (file) void attachImage(file)
    else if (files.length > 0) setError('Drop an image file to attach it.')
  }

  function handleImagePaste(event: ReactClipboardEvent<HTMLFormElement>) {
    const file = pastedImage(event.clipboardData)
    if (file) {
      event.preventDefault()
      void attachImage(file)
    }
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
        body: JSON.stringify({
          title: draft.title,
          body: draft.body,
          tags: draft.tags.map((tag) => tag.name),
          image_data: draft.imageData,
        }),
      })
      if (!response.ok) throw new Error(await readError(response))
      const savedPost = await response.json() as Post
      setPosts((current) => isEditing
        ? current.map((post) => post.id === savedPost.id ? savedPost : post)
        : [savedPost, ...current])
      setSelectedPost(savedPost)
      setEditingPost(undefined)
      setFeedRefresh((current) => current + 1)
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
      setFeedRefresh((current) => current + 1)
      setNotice('Post deleted.')
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Unable to delete the post.')
    } finally {
      setSubmitting(false)
    }
  }

  async function updateTopics(tags: Tag[]) {
    if (!selectedPost) throw new Error('Select a post to change its topics.')
    const response = await fetch(`/api/posts/${selectedPost.id}/tags`, {
      method: 'PUT',
      credentials: 'same-origin',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ tags: tags.map((tag) => tag.name) }),
    })
    if (!response.ok) throw new Error(await readError(response))
    const updated = await response.json() as Post
    setSelectedPost(updated)
    setPosts((current) => current.map((post) => post.id === updated.id ? updated : post))
    setFeedRefresh((current) => current + 1)
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
              setSettingsOpen(false)
              setAdminOpen(false)
              setNotice('')
            }}>Gossip Board</a>
            <div className="account">
              <span className="account-username">@{user.username}</span>
              <span className="account-email">{user.email}</span>
              {user.is_admin && (
                <button
                  className="quiet-button"
                  onClick={() => {
                    setAdminOpen((current) => !current)
                    setSettingsOpen(false)
                    setSelectedPost(null)
                    setEditingPost(undefined)
                    setError('')
                    setNotice('')
                  }}
                  disabled={submitting}
                >
                  {adminOpen ? 'Back to board' : 'Admin overview'}
                </button>
              )}
              <button className="quiet-button" onClick={toggleSettings} disabled={submitting}>
                {settingsOpen ? 'Back to board' : 'Account settings'}
              </button>
              <button className="quiet-button" onClick={logout} disabled={submitting}>Sign out</button>
            </div>
          </header>

          <section className="dashboard-content" aria-labelledby="page-title">
            {settingsOpen ? (
              <>
                <div className="dashboard-heading">
                  <div>
                    <p className="eyebrow">Your account</p>
                    <h1 id="page-title">Account settings</h1>
                  </div>
                </div>
                <form className="post-form" onSubmit={updateUsername}>
                  <p className="settings-description">
                    Your username is how other people find you when sharing posts.
                  </p>
                  <label htmlFor="account-username">Username</label>
                  <input
                    id="account-username"
                    type="text"
                    required
                    maxLength={50}
                    autoComplete="username"
                    value={usernameDraft}
                    onChange={(event) => setUsernameDraft(event.target.value)}
                    disabled={submitting}
                  />
                  <p className="settings-hint">Use up to 50 letters, numbers, dots, hyphens, or underscores.</p>
                  <div className="form-actions">
                    <button className="primary-button" type="submit" disabled={submitting}>
                      {submitting ? 'Saving…' : 'Save username'}
                    </button>
                  </div>
                </form>
                <section className="danger-zone" aria-labelledby="delete-account-title">
                  <h2 id="delete-account-title">Delete account</h2>
                  <p>
                    Permanently remove your account and all your posts, comments, votes, and shares.
                    This action cannot be undone.
                  </p>
                  <button
                    className="danger-button"
                    onClick={() => void deleteAccount()}
                    disabled={submitting}
                  >
                    Delete my account
                  </button>
                </section>
              </>
            ) : adminOpen ? (
              <>
                <div className="dashboard-heading">
                  <div>
                    <p className="eyebrow">Administration</p>
                    <h1 id="page-title">Admin overview</h1>
                  </div>
                  <button
                    className="quiet-button"
                    onClick={() => {
                      setError('')
                      setAdminRefresh((current) => current + 1)
                    }}
                    disabled={adminLoading}
                  >
                    {adminLoading ? 'Refreshing…' : 'Refresh'}
                  </button>
                </div>
                {adminLoading && !adminOverview ? (
                  <p className="empty-state" role="status">Loading admin overview…</p>
                ) : adminOverview ? (
                  <>
                    <div className="admin-stat">
                      <span className="eyebrow">All posts</span>
                      <strong>{adminOverview.post_count.toLocaleString()}</strong>
                    </div>
                    <section className="admin-users" aria-labelledby="admin-users-title">
                      <h2 id="admin-users-title">Users ({adminOverview.users.length})</h2>
                      {adminOverview.users.length === 0 ? (
                        <p className="post-meta">There are no users yet.</p>
                      ) : (
                        <div className="admin-table-wrap">
                          <table className="admin-table">
                            <thead>
                              <tr>
                                <th scope="col">Username</th>
                                <th scope="col">Email</th>
                                <th scope="col">Joined</th>
                              </tr>
                            </thead>
                            <tbody>
                              {adminOverview.users.map((adminUser) => (
                                <tr key={adminUser.id}>
                                  <td>@{adminUser.username}</td>
                                  <td>{adminUser.email}</td>
                                  <td>{formatDate(adminUser.created_at)}</td>
                                </tr>
                              ))}
                            </tbody>
                          </table>
                        </div>
                      )}
                    </section>
                  </>
                ) : (
                  <p className="empty-state">The admin overview could not be loaded.</p>
                )}
              </>
            ) : (
              <>
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

            {!selectedPost && !isEditing && (
              <div className="topic-filter">
                <label htmlFor="topic-filter">Filter by topic</label>
                <select
                  id="topic-filter"
                  value={topicFilter}
                  onChange={(event) => {
                    setTopicFilter(event.target.value)
                    setNotice('')
                  }}
                  disabled={topicsLoading}
                >
                  <option value="">All topics</option>
                  {topicFilter && !topics.some((tag) => tag.name === topicFilter) && (
                    <option value={topicFilter}>{topicFilter}</option>
                  )}
                  {topics.map((tag) => (
                    <option key={tag.id} value={tag.name}>{tag.name}</option>
                  ))}
                </select>
              </div>
            )}

            {isEditing ? (
              <form className="post-form" onSubmit={savePost} onPaste={handleImagePaste}>
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
                <TagPicker
                  value={draft.tags}
                  suggestions={topics}
                  onChange={async (tags) => setDraft((current) => ({ ...current, tags }))}
                  onBusyChange={setTagsSaving}
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
                <div
                  className="image-attachment"
                  onDragOver={(event) => event.preventDefault()}
                  onDrop={handleImageDrop}
                >
                  <label className="image-dropzone" htmlFor="post-image">
                    <strong>Add an image</strong>
                    <span>
                      {imageLoading
                        ? 'Loading image…'
                        : 'Choose a file, drag it here, or paste an image while writing.'}
                    </span>
                    <input
                      className="visually-hidden"
                      id="post-image"
                      type="file"
                      accept="image/png,image/jpeg,image/gif,image/webp"
                      disabled={submitting || imageLoading}
                      onChange={(event) => {
                        const file = event.target.files?.[0]
                        if (file) void attachImage(file)
                        event.target.value = ''
                      }}
                    />
                  </label>
                  {draft.imageData && (
                    <div className="image-preview">
                      <img src={draft.imageData} alt="Image attached to this post" />
                      <button
                        className="quiet-button"
                        type="button"
                        onClick={() => setDraft((current) => ({ ...current, imageData: '' }))}
                        disabled={submitting}
                      >
                        Remove image
                      </button>
                    </div>
                  )}
                </div>
                <div className="form-actions">
                  <button
                    className="primary-button"
                    type="submit"
                    disabled={submitting || tagsSaving || imageLoading}
                  >
                    {submitting ? 'Saving…' : editingPost ? 'Save changes' : 'Publish post'}
                  </button>
                  <button className="quiet-button" type="button" onClick={closeEditor} disabled={submitting || tagsSaving}>
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
                {selectedPost.image_data && (
                  <img className="post-image" src={selectedPost.image_data} alt="" />
                )}
                {selectedPost.author_id === user.id && (
                  <TagPicker
                    key={selectedPost.id}
                    value={selectedPost.tags}
                    suggestions={topics}
                    onChange={updateTopics}
                    onBusyChange={setTagsSaving}
                    disabled={submitting}
                  />
                )}
                {selectedPost.tags.length > 0 && (
                  <div className="topic-labels" aria-label="Topics">
                    {selectedPost.tags.map((tag) => (
                      <button
                        className="topic-label"
                        key={tag.id}
                        onClick={() => {
                          setTopicFilter(tag.name)
                          setSelectedPost(null)
                          setNotice('')
                        }}
                      >
                        {tag.name}
                      </button>
                    ))}
                  </div>
                )}
                <p className="post-body">{selectedPost.body}</p>
                <PostInteractions key={selectedPost.id} postId={selectedPost.id} />
                {selectedPost.author_id === user.id && (
                  <div className="form-actions detail-actions">
                    {sharesLoadedPostId === selectedPost.id && shares.length === 0 && (
                      <button className="secondary-button" onClick={() => startEdit(selectedPost)} disabled={submitting || tagsSaving}>
                        Edit post
                      </button>
                    )}
                    <button className="danger-button" onClick={() => void deletePost(selectedPost)} disabled={submitting || tagsSaving}>
                      Delete post
                    </button>
                  </div>
                )}
                {selectedPost.author_id === user.id
                  && sharesLoadedPostId === selectedPost.id
                  && shares.length > 0 && (
                    <p className="post-meta">Shared post content cannot be edited. You can still change topics or delete your post.</p>
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
                <h2>{topicFilter ? 'No posts in this topic.' : 'Your board is quiet.'}</h2>
                <p>{topicFilter ? 'Try a different topic or show all posts.' : 'Be the first to share a story.'}</p>
                {topicFilter ? (
                  <button className="primary-button" onClick={() => setTopicFilter('')}>Show all topics</button>
                ) : (
                  <button className="primary-button" onClick={startCreate}>Write your first post</button>
                )}
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
                    {post.image_data && (
                      <img className="post-card-image" src={post.image_data} alt="" />
                    )}
                    <span className="post-preview">{post.body}</span>
                    {post.tags.length > 0 && (
                      <span className="topic-labels" aria-label="Topics">
                        {post.tags.map((tag) => (
                          <span className="topic-label" key={tag.id}>{tag.name}</span>
                        ))}
                      </span>
                    )}
                  </button>
                ))}
              </div>
            )}
              </>
            )}
            {notice && !loading && <p className="feedback success" role="status">{notice}</p>}
            {error && <p className="feedback error" role="alert">{error}</p>}
          </section>
        </div>
      )}
    </main>
  )
}
