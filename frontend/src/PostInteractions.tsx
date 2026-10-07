import { type FormEvent, useEffect, useRef, useState } from 'react'
import { readError } from './api'

type VoteSummary = {
  score: number
  your_vote: -1 | 1 | null
}

type PostComment = {
  id: number
  post_id: number
  user_id: number
  username: string
  body: string
  created_at: string
  updated_at: string
}

export default function PostInteractions({ postId }: { postId: number }) {
  const [comments, setComments] = useState<PostComment[]>([])
  const [summary, setSummary] = useState<VoteSummary | null>(null)
  const [body, setBody] = useState('')
  const [loading, setLoading] = useState(true)
  const [submitting, setSubmitting] = useState(false)
  const [loadError, setLoadError] = useState('')
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const [refresh, setRefresh] = useState(0)
  const controllerRef = useRef<AbortController | null>(null)

  useEffect(() => {
    const controller = new AbortController()
    controllerRef.current = controller
    setLoading(true)
    setLoadError('')
    setError('')
    setNotice('')

    async function load() {
      try {
        const detailResponse = await fetch(`/api/posts/${postId}`, {
          credentials: 'same-origin',
          signal: controller.signal,
        })
        if (!detailResponse.ok) throw new Error(await readError(detailResponse))
        const detail = await detailResponse.json() as VoteSummary
        const commentsResponse = await fetch(`/api/posts/${postId}/comments`, {
          credentials: 'same-origin',
          signal: controller.signal,
        })
        if (!commentsResponse.ok) throw new Error(await readError(commentsResponse))
        const result = await commentsResponse.json() as PostComment[]
        if (!controller.signal.aborted) {
          setSummary(detail)
          setComments(result)
        }
      } catch (reason) {
        if (!controller.signal.aborted) {
          setLoadError(reason instanceof Error ? reason.message : 'Unable to load discussion.')
        }
      } finally {
        if (!controller.signal.aborted) setLoading(false)
      }
    }
    void load()

    return () => controller.abort()
  }, [postId, refresh])

  async function addComment(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const controller = controllerRef.current
    if (!controller || controller.signal.aborted) return
    setSubmitting(true)
    setError('')
    setNotice('')
    try {
      const response = await fetch(`/api/posts/${postId}/comments`, {
        method: 'POST',
        credentials: 'same-origin',
        headers: { 'Content-Type': 'application/json' },
        signal: controller.signal,
        body: JSON.stringify({ body }),
      })
      if (!response.ok) throw new Error(await readError(response))
      const comment = await response.json() as PostComment
      if (!controller.signal.aborted) {
        setComments((current) => [...current, comment])
        setBody('')
        setNotice('Comment added.')
      }
    } catch (reason) {
      if (!controller.signal.aborted) {
        setError(reason instanceof Error ? reason.message : 'Unable to add your comment.')
      }
    } finally {
      if (!controller.signal.aborted) setSubmitting(false)
    }
  }

  async function vote(value: -1 | 1) {
    const controller = controllerRef.current
    if (!controller || controller.signal.aborted) return
    setSubmitting(true)
    setError('')
    setNotice('')
    const removing = summary?.your_vote === value
    try {
      const response = await fetch(`/api/posts/${postId}/votes`, {
        method: removing ? 'DELETE' : summary?.your_vote === null ? 'POST' : 'PUT',
        credentials: 'same-origin',
        headers: { 'Content-Type': 'application/json' },
        signal: controller.signal,
        body: removing ? undefined : JSON.stringify({ value }),
      })
      if (!response.ok) throw new Error(await readError(response))
      const result = await response.json() as VoteSummary
      if (!controller.signal.aborted) {
        setSummary(result)
        setNotice(removing ? 'Vote removed.' : value === 1 ? 'Upvote saved.' : 'Downvote saved.')
      }
    } catch (reason) {
      if (!controller.signal.aborted) {
        setError(reason instanceof Error ? reason.message : 'Unable to save your vote.')
      }
    } finally {
      if (!controller.signal.aborted) setSubmitting(false)
    }
  }

  return (
    <section className="discussion-section" aria-labelledby={`discussion-${postId}`}>
      <div className="discussion-heading">
        <h3 id={`discussion-${postId}`}>Discussion</h3>
        <button
          className="quiet-button"
          onClick={() => setRefresh((value) => value + 1)}
          disabled={loading || submitting}
        >
          Refresh discussion
        </button>
      </div>
      {loading ? (
        <p className="post-meta" role="status">Loading comments and votes…</p>
      ) : loadError ? (
        <p className="feedback error" role="alert">{loadError}</p>
      ) : (
        <>
          <div className="vote-controls" aria-label="Post voting">
            <button
              className="quiet-button"
              aria-label="Upvote"
              title="Upvote"
              aria-pressed={summary?.your_vote === 1}
              onClick={() => void vote(1)}
              disabled={submitting}
            >
              <span aria-hidden="true">&#8593;</span>
            </button>
            <span className="vote-score" aria-live="polite">Score: {summary?.score}</span>
            <button
              className="quiet-button"
              aria-label="Downvote"
              title="Downvote"
              aria-pressed={summary?.your_vote === -1}
              onClick={() => void vote(-1)}
              disabled={submitting}
            >
              <span aria-hidden="true">&#8595;</span>
            </button>
          </div>
          <h4>Comments ({comments.length})</h4>
          {comments.length === 0 ? (
            <p className="post-meta">No comments yet. Start the conversation.</p>
          ) : (
            <ol className="comment-list">
              {comments.map((comment) => (
                <li key={comment.id}>
                  <div className="comment-meta">
                    <strong>@{comment.username}</strong>
                    <time className="post-meta" dateTime={comment.created_at}>
                      {new Date(comment.created_at).toLocaleString()}
                    </time>
                  </div>
                  <p className="post-body">{comment.body}</p>
                </li>
              ))}
            </ol>
          )}
          <form className="comment-form" onSubmit={addComment}>
            <label htmlFor={`comment-body-${postId}`}>Add a comment</label>
            <textarea
              id={`comment-body-${postId}`}
              required
              rows={3}
              value={body}
              onChange={(event) => setBody(event.target.value)}
              disabled={submitting}
            />
            <button
              className="primary-button"
              type="submit"
              disabled={submitting || !body.trim()}
            >
              {submitting ? 'Saving…' : 'Post comment'}
            </button>
          </form>
        </>
      )}
      {notice && <p className="feedback success" role="status">{notice}</p>}
      {error && <p className="feedback error" role="alert">{error}</p>}
    </section>
  )
}
