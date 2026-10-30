import logo from "../assets/logo.svg";
import styles from "./Header.module.css";

export interface HeaderProps {
  onLogout?: () => void;
  elevated?: boolean;
}

export function Header({ onLogout, elevated = false }: HeaderProps) {
  return (
    <header className={elevated ? styles.elevated : styles.header}>
      <div className={styles.brand}>
        <img src={logo} alt="" className={styles.logo} />
        <span>spoticlean</span>
      </div>
      {onLogout && (
        <button type="button" className={styles.logout} onClick={onLogout}>
          Se déconnecter
        </button>
      )}
    </header>
  );
}
