<#import "template.ftl" as layout>
<#-- Two cases on one page:
     1. Sign in: "Sign in with Google" first (the owner's normal way in; shown only when the realm offers the IdP), then
        email + password. No registration link (registrationAllowed: false). The email field is NOT autofocused: in the
        phone's sheet the keyboard would cover the Google button.
     2. Re-authentication while linking an IdP (usernameHidden). Since the Google first login auto-links
        (idp-auto-link), this no longer happens for Google; kept so any idp-username-password-form still looks right. -->
<#assign linking = usernameHidden??>
<#assign bad = messagesPerField.existsError('username','password')>
<#if linking>
    <#global pageTitle = "Link your Google account">
    <#global pageSubtitle = "Enter your Punchable password once. After that, Google signs you straight in.">
<#else>
    <#global pageTitle = "Sign in">
    <#global pageSubtitle = "Every bit you've written, and how long they laughed.">
</#if>
<@layout.registrationLayout displayMessage=!bad displayInfo=false; section>
    <#if section = "socialProviders">
        <#if !linking && social?? && social.providers?has_content>
            <#list social.providers as p>
                <#if p.alias == "google">
                    <a id="social-google" class="pk-google" href="${p.loginUrl}">
                        <svg viewBox="0 0 24 24" aria-hidden="true">
                            <path fill="#4285F4" d="M22.56 12.25c0-.78-.07-1.53-.2-2.25H12v4.26h5.92c-.26 1.37-1.04 2.53-2.21 3.31v2.77h3.57c2.08-1.92 3.28-4.74 3.28-8.09z"/>
                            <path fill="#34A853" d="M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84C3.99 20.53 7.7 23 12 23z"/>
                            <path fill="#FBBC05" d="M5.84 14.09c-.22-.66-.35-1.36-.35-2.09s.13-1.43.35-2.09V7.07H2.18C1.43 8.55 1 10.22 1 12s.43 3.45 1.18 4.93l3.66-2.84z"/>
                            <path fill="#EA4335" d="M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1 7.7 1 3.99 3.47 2.18 7.07l3.66 2.84c.87-2.6 3.3-4.53 6.16-4.53z"/>
                        </svg>
                        <span>Sign in with Google</span>
                    </a>
                <#else>
                    <a id="social-${p.alias}" class="pk-google pk-social-other" href="${p.loginUrl}"><span>Sign in with ${p.displayName!p.alias}</span></a>
                </#if>
            </#list>
            <#if realm.password><div class="pk-or" role="separator" aria-label="or">or</div></#if>
        </#if>
    <#elseif section = "form">
        <#if realm.password>
            <form id="kc-form-login" action="${url.loginAction}" method="post" data-once novalidate>
                <#if !linking>
                    <div class="pk-field">
                        <label for="username">Email</label>
                        <input id="username" name="username" type="email" inputmode="email" autocomplete="username" autocapitalize="none" autocorrect="off" spellcheck="false"
                            value="${(login.username!'')}" placeholder="you@example.com"
                            <#if bad>aria-invalid="true" aria-describedby="input-error"</#if> />
                    </div>
                <#else>
                    <p class="pk-hint">Signing in as <b>${(auth.attemptedUsername)!((login.username)!'')}</b></p>
                </#if>
                <div class="pk-field">
                    <label for="password">Password</label>
                    <div class="pk-pw">
                        <input id="password" name="password" type="password" autocomplete="current-password" autocapitalize="none" autocorrect="off" spellcheck="false" <#if linking>autofocus</#if>
                            <#if bad>aria-invalid="true" aria-describedby="input-error"</#if> />
                        <@layout.pwToggle field="password"/>
                    </div>
                    <#if bad>
                        <span id="input-error" class="pk-err" role="alert">${kcSanitize(messagesPerField.getFirstError('username','password'))?no_esc}</span>
                    </#if>
                </div>
                <div class="pk-row">
                    <#if realm.rememberMe && !linking>
                        <label class="pk-check"><input id="rememberMe" name="rememberMe" type="checkbox" <#if login.rememberMe??>checked</#if>> Remember me</label>
                    </#if>
                    <#if realm.resetPasswordAllowed>
                        <a class="pk-link" href="${url.loginResetCredentialsUrl}">Forgot password?</a>
                    </#if>
                </div>
                <input type="hidden" id="id-hidden-input" name="credentialId" <#if auth.selectedCredential?has_content>value="${(auth.selectedCredential)!''}"</#if>/>
                <button class="pk-btn" name="login" id="kc-login" type="submit" data-busy="Signing in…"><#if linking>Link Google<#else>Sign in</#if></button>
            </form>
        </#if>
    </#if>
</@layout.registrationLayout>
