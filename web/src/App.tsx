import { useEffect, useState } from "react";
import { getSession, logout, ProblemError, type ScanResult } from "./api/client";
import { LoginScreen } from "./components/LoginScreen";
import { ReviewScreen } from "./components/ReviewScreen";
import { ScanScreen } from "./components/ScanScreen";
import { logger } from "./logger";
import { authErrorMessage, problemMessage } from "./messages";

type Phase =
  | { kind: "checking" }
  | { kind: "login"; message: string | null }
  | { kind: "scanning" }
  | { kind: "review"; result: ScanResult; scannedAt: Date };

const SESSION_EXPIRED = "Ta session a expiré. Reconnecte-toi pour continuer.";

const failureMessage = (error: unknown) =>
  error instanceof ProblemError
    ? problemMessage(error.problem)
    : "Une erreur inattendue s'est produite.";

/**
 * Removes `authError` from the address bar, so that a reload does not show
 * it again, and returns it.
 */
export function takeAuthError(): string | null {
  const url = new URL(window.location.href);
  const code = url.searchParams.get("authError");
  if (code !== null) {
    url.searchParams.delete("authError");
    window.history.replaceState(
      window.history.state,
      "",
      `${url.pathname}${url.search}${url.hash}`,
    );
  }
  return code;
}

export interface AppProps {
  authError: string | null;
}

export function App({ authError }: AppProps) {
  const [phase, setPhase] = useState<Phase>({ kind: "checking" });

  useEffect(() => {
    let active = true;
    getSession()
      .then((session) => {
        if (active) {
          setPhase(
            session
              ? { kind: "scanning" }
              : { kind: "login", message: authError && authErrorMessage(authError) },
          );
        }
      })
      .catch((error: unknown) => {
        if (active) {
          setPhase({ kind: "login", message: failureMessage(error) });
        }
      });
    return () => {
      active = false;
    };
  }, [authError]);

  const expired = () => setPhase({ kind: "login", message: SESSION_EXPIRED });
  const logOut = async () => {
    try {
      await logout();
      setPhase({ kind: "login", message: null });
    } catch (error) {
      logger.warn("logout.failed", { error: String(error) });
      setPhase({ kind: "login", message: failureMessage(error) });
    }
  };

  switch (phase.kind) {
    case "checking":
      return null;
    case "login":
      return <LoginScreen message={phase.message} />;
    case "scanning":
      return (
        <ScanScreen
          onResult={(result) => setPhase({ kind: "review", result, scannedAt: new Date() })}
          onUnauthenticated={expired}
          onLogout={() => void logOut()}
        />
      );
    case "review":
      return (
        <ReviewScreen
          result={phase.result}
          scannedAt={phase.scannedAt}
          onRescan={() => setPhase({ kind: "scanning" })}
          onLogout={() => void logOut()}
          onUnauthenticated={expired}
        />
      );
  }
}
