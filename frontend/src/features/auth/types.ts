import type { Role } from "@/features/spaces/types";

/** `GET /api/auth/me` */
export interface Me {
  user: {
    id: string;
    email: string;
    displayName: string;
    emailVerified: boolean;
    mfaEnabled: boolean;
    /** False for an account created through Google that has not set a password yet. */
    hasPassword: boolean;
    googleLinked: boolean;
  };
  /** Null on the base host. */
  space: { slug: string; name: string; role: Role } | null;
}

export interface LoginBody {
  email: string;
  password: string;
}

export interface RegisterBody {
  email: string;
  password: string;
  displayName: string;
  /** Language of the verification e-mail. */
  locale: string;
}

/** `POST /api/auth/login` 200: the password was right, a TOTP / recovery code must follow. */
export interface LoginMfaRequired {
  mfa: "required";
}

export interface ChangePasswordBody {
  /** Omitted only to set the first password of a Google-only account (within 10 min of a Google login). */
  currentPassword?: string;
  newPassword: string;
  /** Step-up, only for a user with TOTP. */
  code?: string;
}

/** `GET /api/account/mfa` */
export interface MfaStatus {
  enabled: boolean;
  recoveryCodesLeft: number;
  /** The user's spaces whose policy requires TOTP. */
  requiredBy: { slug: string; name: string }[];
}

/** `POST /api/account/mfa/setup` */
export interface MfaSetup {
  /** Base32, for manual entry. */
  secret: string;
  otpauthUri: string;
}

/** `POST /api/account/mfa/enable` and `/recovery-codes`: shown once, never stored by the client. */
export interface RecoveryCodes {
  recoveryCodes: string[];
}

/** `POST /api/account/mfa/disable` and `/recovery-codes`. */
export interface MfaConfirmBody {
  password: string;
  code: string;
}

/** `POST /api/account/google/link` 200: the Google start URL to navigate to. */
export interface GoogleLinkStart {
  url: string;
}

/** `DELETE /api/account/google` and `POST /api/account/google/link` (password re-confirmation). */
export interface UnlinkGoogleBody {
  password: string;
  /** Step-up, only for a user with TOTP. */
  code?: string;
}
