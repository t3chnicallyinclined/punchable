<#import "template.ftl" as layout>
<#-- Required action UPDATE_PASSWORD (after a reset link, or an admin "set your password"). Field names and the error
     ids are the base theme's (password-new, password-confirm, logout-sessions, cancel-aia). The realm's policy is
     length(10): the hint says so; the server is the judge. -->
<#global pageTitle = "Set a new password">
<#global pageSubtitle = "At least 10 characters. Length beats symbols.">
<#assign badNew = messagesPerField.existsError('password')>
<#assign badConfirm = messagesPerField.existsError('password-confirm')>
<@layout.registrationLayout displayMessage=!(badNew || badConfirm) displayInfo=false; section>
    <#if section = "form">
        <form id="kc-passwd-update-form" action="${url.loginAction}" method="post" data-once novalidate>
            <input type="text" id="username" name="username" value="${(username)!''}" autocomplete="username" readonly hidden />
            <div class="pk-field">
                <label for="password-new">New password</label>
                <div class="pk-pw">
                    <input id="password-new" name="password-new" type="password" autocomplete="new-password" autocapitalize="none" autocorrect="off" spellcheck="false" autofocus
                        <#if badNew>aria-invalid="true" aria-describedby="input-error-password"</#if> />
                    <@layout.pwToggle field="password-new"/>
                </div>
                <#if badNew>
                    <span id="input-error-password" class="pk-err" role="alert">${kcSanitize(messagesPerField.get('password'))?no_esc}</span>
                </#if>
            </div>
            <div class="pk-field">
                <label for="password-confirm">Confirm password</label>
                <div class="pk-pw">
                    <input id="password-confirm" name="password-confirm" type="password" autocomplete="new-password" autocapitalize="none" autocorrect="off" spellcheck="false"
                        <#if badConfirm>aria-invalid="true" aria-describedby="input-error-password-confirm"</#if> />
                    <@layout.pwToggle field="password-confirm"/>
                </div>
                <#if badConfirm>
                    <span id="input-error-password-confirm" class="pk-err" role="alert">${kcSanitize(messagesPerField.get('password-confirm'))?no_esc}</span>
                </#if>
            </div>
            <label class="pk-check"><input type="checkbox" id="logout-sessions" name="logout-sessions" value="on" checked> Sign out of other devices</label>
            <button class="pk-btn" type="submit" data-busy="Saving…">Save password</button>
            <#if isAppInitiatedAction??>
                <button class="pk-btn pk-btn-quiet" type="submit" name="cancel-aia" value="true" formnovalidate>Not now</button>
            </#if>
        </form>
    </#if>
</@layout.registrationLayout>
