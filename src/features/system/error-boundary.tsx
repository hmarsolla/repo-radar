import { Component, type ErrorInfo, type ReactNode } from "react";
import { AlertTriangle, ClipboardCopy, RotateCcw } from "lucide-react";

import { Button } from "@/components/ui/button";
import { reportFrontendError } from "@/lib/report-error";

interface Props {
  children: ReactNode;
  /** Names the subtree, so the log says *where* the failure happened. */
  context: string;
  /**
   * When set, a failure replaces only this subtree with a compact inline
   * notice and the rest of the app keeps working. Without it, the failure
   * takes over the window.
   */
  inline?: boolean;
}

interface State {
  error: Error | null;
  componentStack: string | null;
}

/**
 * Catches render-time exceptions below it.
 *
 * React unmounts the **entire** tree when a render throws and no boundary
 * catches it — the window goes blank with no message, which is
 * indistinguishable from the process dying. repo-radar had no boundary
 * anywhere, so a single bad value from the backend (a `null` where the
 * generated bindings promised a number, an empty array indexed at `[0]`)
 * blanked the whole app.
 *
 * Wrapping routes individually (`inline`) keeps one broken view from taking
 * the navigation with it, so the user can still get back to a working screen.
 */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null, componentStack: null };

  static getDerivedStateFromError(error: Error): Partial<State> {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    this.setState({ componentStack: info.componentStack ?? null });
    reportFrontendError(this.props.context, error, info.componentStack);
  }

  private reset = () => {
    this.setState({ error: null, componentStack: null });
  };

  private copy = () => {
    const { error, componentStack } = this.state;
    const text = [
      `context: ${this.props.context}`,
      `error: ${error?.message ?? "unknown"}`,
      error?.stack ? `\nstack:\n${error.stack}` : "",
      componentStack ? `\ncomponent stack:${componentStack}` : "",
    ].join("\n");
    void navigator.clipboard?.writeText(text);
  };

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;

    const details = (
      <>
        <pre className="mt-2 max-h-40 overflow-auto rounded-md bg-secondary p-2 text-xs whitespace-pre-wrap">
          {error.message}
          {this.state.componentStack}
        </pre>
        <div className="mt-3 flex flex-wrap items-center gap-2">
          <Button variant="outline" size="sm" onClick={this.reset}>
            <RotateCcw />
            Try again
          </Button>
          <Button
            variant="outline"
            size="sm"
            onClick={() => window.location.reload()}
          >
            Reload app
          </Button>
          <Button variant="ghost" size="sm" onClick={this.copy}>
            <ClipboardCopy />
            Copy details
          </Button>
        </div>
      </>
    );

    if (this.props.inline) {
      return (
        <div className="m-4 rounded-lg border border-warn/40 bg-warn/5 p-4">
          <div className="flex items-center gap-2 text-warn">
            <AlertTriangle className="size-4" />
            <h2 className="text-sm font-semibold">This view failed to load</h2>
          </div>
          <p className="mt-1 text-xs text-muted-foreground">
            The rest of Repo Radar still works — the details below are also in{" "}
            <code>logs/repo-radar.log</code>.
          </p>
          {details}
        </div>
      );
    }

    return (
      <div className="flex min-h-screen items-center justify-center bg-background p-6">
        <div className="w-full max-w-lg rounded-xl border border-warn/40 bg-card p-6">
          <div className="flex items-center gap-2 text-warn">
            <AlertTriangle className="size-5" />
            <h1 className="text-lg font-semibold">Something went wrong</h1>
          </div>
          <p className="mt-2 text-sm text-muted-foreground">
            Repo Radar hit an unexpected error while drawing the interface. Your
            data is untouched — this is a display failure, not a lost scan.
          </p>
          {details}
        </div>
      </div>
    );
  }
}
