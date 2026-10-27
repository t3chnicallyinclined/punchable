<#import "template.ftl" as layout>
<#global pageTitle = "Reset your password">
<#global pageSubtitle = "Enter your email. We'll send a link to set a new password.">
<@layout.registrationLayout displayMessage=!messagesPerField.existsError('username') displayInfo=false; section>
    <#if section = "form">
        <form id="kc-reset-password-form" action="${url.loginAction}" method="post">
            <div class="pk-field">
                <label for="username">Email</label>
                <input id="username" name="username" type="email" inputmode="email" autocomplete="username" autocapitalize="none" spellcheck="false" autofocus
                    value="${(auth.attemptedUsername)!''}" placeholder="you@example.com"
                    <#if messagesPerField.existsError('username')>aria-invalid="true" aria-describedby="pk-user-err"</#if> />
                <#if messagesPerField.existsError('username')>
                    <span id="pk-user-err" class="pk-err" role="alert">${kcSanitize(messagesPerField.get('username'))?no_esc}</span>
                </#if>
            </div>
            <button class="pk-btn" type="submit">Email me a link</button>
            <a class="pk-link" href="${url.loginUrl}">Back to sign in</a>
        </form>
    </#if>
</@layout.registrationLayout>
