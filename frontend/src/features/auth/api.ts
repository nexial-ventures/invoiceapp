import { request } from "@/api/client";
import type { ChangePasswordBody, GoogleLinkStart, LoginBody, LoginMfaRequired, Me, MfaConfirmBody, MfaSetup, MfaStatus, RecoveryCodes, RegisterBody, UnlinkGoogleBody } from "./types";

export const authApi = {
  // A 401 is the expected "not signed in" / "wrong credentials" / "wrong code" answer for these.
  me: () => request<Me>("/api/auth/me", { quiet401: true }),
  /** Undefined (204) = signed in; `{ mfa: "required" }` = the code step follows. */
  login: (body: LoginBody) => request<LoginMfaRequired | undefined>("/api/auth/login", { method: "POST", body, quiet401: true }),
  loginMfa: (code: string) => request<void>("/api/auth/login/mfa", { method: "POST", body: { code }, quiet401: true }),
  logout: () => request<void>("/api/auth/logout", { method: "POST", quiet401: true }),
  revokeOthers: () => request<void>("/api/auth/sessions/revoke-others", { method: "POST" }),
  register: (body: RegisterBody) => request<void>("/api/auth/register", { method: "POST", body }),
  verify: (token: string) => request<void>("/api/auth/verify", { method: "POST", body: { token } }),
  resendVerification: (email: string, locale: string) =>
    request<void>("/api/auth/verify/resend", { method: "POST", body: { email, locale } }),
  requestReset: (email: string, locale: string) =>
    request<void>("/api/auth/password-reset", { method: "POST", body: { email, locale } }),
  confirmReset: (token: string, password: string) =>
    request<void>("/api/auth/password-reset/confirm", { method: "POST", body: { token, password } }),
  changePassword: (body: ChangePasswordBody) => request<void>("/api/account/password", { method: "POST", body }),
  linkGoogle: (body: UnlinkGoogleBody) => request<GoogleLinkStart>("/api/account/google/link", { method: "POST", body }),
  unlinkGoogle: (body: UnlinkGoogleBody) => request<void>("/api/account/google", { method: "DELETE", body }),
};

export const mfaApi = {
  status: () => request<MfaStatus>("/api/account/mfa"),
  setup: (password: string) => request<MfaSetup>("/api/account/mfa/setup", { method: "POST", body: { password } }),
  enable: (code: string) => request<RecoveryCodes>("/api/account/mfa/enable", { method: "POST", body: { code } }),
  disable: (body: MfaConfirmBody) => request<void>("/api/account/mfa/disable", { method: "POST", body }),
  regenerate: (body: MfaConfirmBody) => request<RecoveryCodes>("/api/account/mfa/recovery-codes", { method: "POST", body }),
};
