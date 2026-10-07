import { type KeyboardEvent, useEffect, useId, useState } from 'react'
import { readError } from './api'

export type Tag = {
  id: number
  name: string
  slug: string
}

type Props = {
  value: Tag[]
  suggestions: Tag[]
  onChange: (tags: Tag[]) => Promise<void>
  onBusyChange: (busy: boolean) => void
  disabled?: boolean
}

export default function TagPicker({ value, suggestions, onChange, onBusyChange, disabled = false }: Props) {
  const id = useId()
  const [input, setInput] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [catalog, setCatalog] = useState<Tag[]>([])
  const [catalogError, setCatalogError] = useState('')
  useEffect(() => {
    const controller = new AbortController()
    void fetch('/api/tags?catalog=true', {
      credentials: 'same-origin',
      signal: controller.signal,
    })
      .then(async (response) => {
        if (!response.ok) throw new Error(await readError(response))
        return response.json() as Promise<Tag[]>
      })
      .then((tags) => {
        if (!controller.signal.aborted) setCatalog(tags)
      })
      .catch((reason: unknown) => {
        if (!controller.signal.aborted) {
          setCatalogError(reason instanceof Error ? reason.message : 'Unable to load topic suggestions.')
        }
      })
    return () => controller.abort()
  }, [])
  const allSuggestions = [...new Map([...catalog, ...suggestions, ...value].map((tag) => [tag.id, tag])).values()]
  const normalized = input.trim().toLowerCase().split(/[\s-]+/).filter(Boolean).join(' ')
  const existing = allSuggestions.find((tag) => tag.name === normalized)
  const available = allSuggestions.filter((tag) => !value.some((selected) => selected.id === tag.id))

  async function addTag() {
    if (!input.trim() || busy || disabled) return
    setBusy(true)
    onBusyChange(true)
    setError('')
    try {
      let tag = existing
      if (!tag) {
        const response = await fetch('/api/tags', {
          method: 'POST',
          credentials: 'same-origin',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ name: input }),
        })
        if (!response.ok) throw new Error(await readError(response))
        tag = await response.json() as Tag
      }
      if (!value.some((selected) => selected.id === tag.id)) {
        await onChange([...value, tag])
      }
      setCatalog((current) => current.some((selected) => selected.id === tag.id) ? current : [...current, tag])
      setInput('')
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Unable to add this topic.')
    } finally {
      setBusy(false)
      onBusyChange(false)
    }
  }

  async function removeTag(tag: Tag) {
    setBusy(true)
    onBusyChange(true)
    setError('')
    try {
      await onChange(value.filter((selected) => selected.id !== tag.id))
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Unable to remove this topic.')
    } finally {
      setBusy(false)
      onBusyChange(false)
    }
  }

  function handleKeyDown(event: KeyboardEvent<HTMLInputElement>) {
    if (event.key === 'Enter') {
      event.preventDefault()
      void addTag()
    }
  }

  return (
    <div className="tag-picker">
      <label htmlFor={id}>Topics</label>
      <div className="topic-labels" aria-label="Selected topics">
        {value.map((tag) => (
          <span className="topic-label" key={tag.id}>
            {tag.name}
            <button
              type="button"
              className="remove-topic"
              aria-label={`Remove ${tag.name}`}
              disabled={disabled || busy}
              onClick={() => void removeTag(tag)}
            >
              &times;
            </button>
          </span>
        ))}
      </div>
      <div className="tag-picker-input">
        <input
          id={id}
          value={input}
          onChange={(event) => setInput(event.target.value)}
          onKeyDown={handleKeyDown}
          list={`${id}-suggestions`}
          aria-describedby={`${id}-help`}
          autoComplete="off"
          placeholder="Find or create a topic"
          disabled={disabled || busy}
        />
        <datalist id={`${id}-suggestions`}>
          {available.map((tag) => <option key={tag.id} value={tag.name} />)}
        </datalist>
        <button
          type="button"
          className="quiet-button"
          disabled={disabled || busy || !input.trim()}
          onClick={() => void addTag()}
        >
          {busy ? 'Saving...' : existing ? 'Add topic' : 'Create topic'}
        </button>
      </div>
      <p id={`${id}-help`} className="post-meta">
        Add topics one at a time. Select a suggestion or create a new topic, then press Enter or use the button.
      </p>
      {error && <p className="feedback error" role="alert">{error}</p>}
      {catalogError && <p className="feedback error" role="alert">{catalogError}</p>}
    </div>
  )
}
