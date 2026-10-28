<#import "template.ftl" as layout>
<#global pageTitle = "Reset your password">
<#global pageSubtitle = "Enter your email and we'll send you a link to set a new one.">
<#assign bad = messagesPerField.existsError('username')>
<@layout.registrationLayout displayMessage=!bad displayInfo=false; section>
    <#if section = "form">
        <form id="kc-reset-password-form" action="${url.loginAction}" method="post" data-once novalidate>
            <div class="pk-field">
                <label for="username">Email</label>
                <input id="username" name="username" type="email" inputmode="email" autocomplete="username" autocapitalize="none" autocorrect="off" spellcheck="false" autofocus
                    value="${(auth.attemptedUsername)!''}" placeholder="you@example.com"
                    <#if bad>aria-invalid="true" aria-describedby="input-error-username"</#if> />
                <#if bad>
                    <span id="input-error-username" class="pk-err" role="alert">${kcSanitize(messagesPerField.get('username'))?no_esc}</span>
                </#if>
            </div>
            <button class="pk-btn" type="submit" data-busy="Sending…">Send link</button>
            <a class="pk-link pk-center" href="${url.loginUrl}">Back to sign in</a>
        </form>
    </#if>
</@layout.registrationLayout>
