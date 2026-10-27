<#import "template.ftl" as layout>
<#global pageTitle = "Check your email">
<@layout.registrationLayout displayInfo=false; section>
    <#if section = "form">
        <p class="pk-sub" style="margin:0">To link ${idpDisplayName!'Google'}, open the link we sent to <b>${(brokerContext.username)!'your address'}</b>.</p>
        <p class="pk-hint">No email? Check spam, or send it again.</p>
        <a class="pk-btn pk-btn-quiet" href="${url.loginAction}?resend=true">Send the link again</a>
        <a class="pk-link" href="${url.loginRestartFlowUrl}">Cancel</a>
    </#if>
</@layout.registrationLayout>
