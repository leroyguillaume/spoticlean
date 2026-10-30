import { type Problem, problemCode, type RemovalFailureReason } from "./api/client";

const AUTH_ERRORS: Record<string, string> = {
  invalidState:
    "La connexion a expiré ou a été lancée depuis un autre onglet. Relance-la depuis cette page.",
  accessDenied: "Tu as refusé l'accès à Spotify : spoticlean en a besoin pour lire tes likes.",
  loginFailed:
    "Spotify a refusé la connexion. Vérifie que ton compte est autorisé dans l'app Spotify, puis réessaie.",
  spotifyUnavailable: "Spotify ne répond pas. Réessaie dans un instant.",
};

/** The message for an `authError` the login callback redirected with. */
export function authErrorMessage(code: string): string {
  return AUTH_ERRORS[code] ?? "La connexion à Spotify a échoué. Réessaie.";
}

const PROBLEMS: Record<string, string> = {
  spotifyRateLimited: "Spotify limite les requêtes. Réessaie dans quelques minutes.",
  spotifyForbidden:
    "Spotify refuse l'accès à ta bibliothèque. Vérifie que ton compte est autorisé dans l'app Spotify.",
  spotifyUnauthorized: "Spotify a refusé ta session. Reconnecte-toi.",
  spotifyUnavailable: "Spotify ne répond pas correctement. Réessaie dans un instant.",
  spotifyRejected: "Spotify a refusé la demande.",
  unreachable: "Impossible de joindre spoticlean. Vérifie ta connexion, puis réessaie.",
  disconnected: "La connexion au serveur a été coupée.",
};

export function problemMessage(problem: Problem): string {
  return PROBLEMS[problemCode(problem)] ?? "Une erreur inattendue s'est produite.";
}

export const FAILURE_REASONS: Record<RemovalFailureReason, string> = {
  spotifyUnauthorized: "Spotify a refusé ta session",
  spotifyForbidden: "Spotify a refusé l'accès",
  spotifyRateLimited: "Spotify limite les requêtes",
  spotifyUnavailable: "Spotify n'a pas répondu",
  spotifyRejected: "Spotify a refusé ce lot",
  skipped: "pas envoyé, après l'échec d'un lot précédent",
};
