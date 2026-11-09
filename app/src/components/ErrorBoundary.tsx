import { Component, type ErrorInfo, type ReactNode } from "react"
import { Button } from "@/components/ui/button"

/** Keeps one broken view from blanking the window: says what went wrong, with a way back. */
export class ErrorBoundary extends Component<{ children: ReactNode; label: string }, { error: Error | null }> {
  state: { error: Error | null } = { error: null }

  static getDerivedStateFromError(error: Error) {
    return { error }
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error(`oah: ${this.props.label} failed`, error, info.componentStack)
  }

  render() {
    if (!this.state.error) return this.props.children
    return (
      <div role="alert" className="flex h-full flex-col items-center justify-center gap-3 p-6 text-center">
        <p className="text-[13px] font-medium">Something went wrong showing {this.props.label}.</p>
        <p className="max-w-md font-mono text-[11.5px] break-all text-muted-foreground">{this.state.error.message}</p>
        <div className="flex gap-2">
          <Button size="sm" variant="outline" onClick={() => this.setState({ error: null })}>
            Try again
          </Button>
          <Button size="sm" variant="ghost" onClick={() => location.reload()}>
            Reload oah
          </Button>
        </div>
      </div>
    )
  }
}
