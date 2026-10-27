<#import "template.ftl" as layout>
<#global pageTitle = "Set a new password">
<#global pageSubtitle = "At least 10 characters. Length beats symbols: a line you'd never say on stage works fine.">
<@layout.registrationLayout displayMessage=!messagesPerField.existsError('password','password-confirm') displayInfo=false; section>
    <#if section = "form">
        <form id="kc-passwd-update-form" action="${url.loginAction}" method="post">
            <input type="text" id="username" name="username" value="${(username)!''}" autocomplete="username" readonly style="display:none" />
            <div class="pk-field">
                <label for="password-new">New password</label>
                <input id="password-new" name="password-new" type="password" autocomplete="new-password" minlength="10" autofocus
                    <#if messagesPerField.existsError('password','password-confirm')>aria-invalid="true"</#if> />
                <#if messagesPerField.existsError('password')>
                    <span class="pk-err" role="alert">${kcSanitize(messagesPerField.get('password'))?no_esc}</span>
                </#if>
            </div>
            <div class="pk-field">
                <label for="password-confirm">Type it again</label>
                <input id="password-confirm" name="password-confirm" type="password" autocomplete="new-password"
                    <#if messagesPerField.existsError('password-confirm')>aria-invalid="true"</#if> />
                <#if messagesPerField.existsError('password-confirm')>
                    <span class="pk-err" role="alert">${kcSanitize(messagesPerField.get('password-confirm'))?no_esc}</span>
                </#if>
            </div>
            <label class="pk-check"><input type="checkbox" id="logout-sessions" name="logout-sessions" value="on" checked> Sign out on my other devices</label>
            <button class="pk-btn" type="submit">Save password</button>
            <#if isAppInitiatedAction??>
                <button class="pk-btn pk-btn-quiet" type="submit" name="cancel-aia" value="true" formnovalidate>Keep my old password</button>
            </#if>
        </form>
    </#if>
</@layout.registrationLayout>
