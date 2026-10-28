<#import "template.ftl" as layout>
<#-- "Verify by email" step of a first-broker flow. The resend is the base page's plain link to loginAction. -->
<#global pageTitle = "Check your email">
<@layout.registrationLayout displayInfo=false; section>
    <#if section = "form">
        <p class="pk-hint">To link ${idpDisplayName!'Google'}, open the link we sent to <b>${(brokerContext.username)!'your address'}</b>.</p>
        <p class="pk-hint">No email? Check spam, or send it again.</p>
        <a class="pk-btn pk-btn-quiet" href="${url.loginAction}">Send again</a>
        <a class="pk-link pk-center" href="${url.loginRestartFlowUrl}">Cancel</a>
    </#if>
</@layout.registrationLayout>
