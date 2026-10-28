<#import "template.ftl" as layout>
<#-- Required action VERIFY_EMAIL. Like the base page: `verifyEmail` is set when the link has been sent (to that
     address) and the form re-sends it; otherwise the form sends it. Both are a POST to loginAction. -->
<#global pageTitle = "Check your email">
<@layout.registrationLayout displayInfo=false; section>
    <#if section = "form">
        <#if verifyEmail??>
            <p class="pk-hint">We sent a link to <b>${verifyEmail}</b>. Open it to confirm this address, then come back here.</p>
            <p class="pk-hint">Nothing there after a few minutes? Check spam, or send it again.</p>
        <#else>
            <p class="pk-hint">To continue, confirm <b>${(user.email)!'your email address'}</b>. We'll send you a link.</p>
        </#if>
        <form id="kc-verify-email-form" action="${url.loginAction}" method="post" data-once>
            <button class="pk-btn<#if verifyEmail??> pk-btn-quiet</#if>" type="submit" data-busy="Sending…"><#if verifyEmail??>Send again<#else>Send the link</#if></button>
            <#if isAppInitiatedAction??>
                <button class="pk-btn pk-btn-quiet" type="submit" name="cancel-aia" value="true" formnovalidate>Not now</button>
            <#else>
                <a class="pk-link pk-center" href="${url.loginRestartFlowUrl}">Back to sign in</a>
            </#if>
        </form>
    </#if>
</@layout.registrationLayout>
