/* eslint-disable no-undef */
'use client'

export default function Root({ children }) {
  return (
    <html>
      <head></head>
      <button onClick={() => next.router.push('/a')}>a</button>
      <button onClick={() => next.router.push('/b')}>a</button>
      <body>{children}</body>
    </html>
  )
}
