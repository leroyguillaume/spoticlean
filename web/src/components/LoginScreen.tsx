import { LOGIN_URL } from "../api/client";
import { Header } from "./Header";
import styles from "./LoginScreen.module.css";
import ui from "./ui.module.css";

export interface LoginScreenProps {
  message: string | null;
}

export function LoginScreen({ message }: LoginScreenProps) {
  return (
    <>
      <title>Connexion · spoticlean</title>
      <Header />
      <main className={styles.main}>
        <section className={styles.intro}>
          <h1 className={styles.title}>Fais le ménage dans tes titres likés.</h1>
          <p className={styles.lead}>
            spoticlean repère les titres que tu as likés plusieurs fois : remasters, rééditions,
            compilations, versions avec un invité. Tu choisis lequel garder, groupe par groupe.
          </p>
          {message && (
            <p role="alert" className={ui.alert}>
              {message}
            </p>
          )}
          <a href={LOGIN_URL} className={styles.login}>
            Se connecter avec Spotify
          </a>
          <p className={styles.note}>
            spoticlean lit tes titres likés et ne retire que ceux que tu valides.
          </p>
        </section>
        <section aria-label="Aperçu d'un groupe de doublons" className={styles.preview}>
          <div className={styles.reason}>
            <span className={ui.certain}>Certain</span>
            <span>Même ISRC</span>
          </div>
          <div>
            <div className={styles.song}>Les Heures claires</div>
            <div className={styles.artist}>Orchestre du Dimanche</div>
          </div>
          <div className={styles.rows}>
            <div className={styles.kept}>
              <span className={styles.action}>Garder</span>
              <span className={styles.album}>Premier Soir</span>
              <span className={styles.year}>1994</span>
            </div>
            <div className={styles.row}>
              <span className={styles.action}>Retirer</span>
              <span className={styles.album}>Premier Soir (Remastered 2014)</span>
              <span className={styles.year}>1994</span>
            </div>
          </div>
        </section>
      </main>
    </>
  );
}
