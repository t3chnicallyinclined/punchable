<#import "template.ftl" as layout>
<#global pageTitle = "Check your email">
<@layout.registrationLayout displayInfo=false; section>
    <#if section = "form">
        <p class="pk-sub" style="margin:0">We sent a link to <b>${(user.email)!'your email'}</b>. Open it to confirm this address, then come back here.</p>
        <p class="pk-hint">Nothing there after a few minutes? Check spam, or send it again.</p>
        <a class="pk-btn pk-btn-quiet" href="${url.loginAction}">Send the link again</a>
        <a class="pk-link" href="${url.loginRestartFlowUrl}">Back to sign in</a>
    </#if>
</@layout.registrationLayout>
