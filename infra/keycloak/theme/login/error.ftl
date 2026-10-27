<#import "template.ftl" as layout>
<#global pageTitle = "That didn't work">
<@layout.registrationLayout displayMessage=false; section>
    <#if section = "form">
        <div class="pk-alert pk-alert-error" role="alert">${kcSanitize((message.summary)!'The sign-in server hit an error.')?no_esc}</div>
        <#if !skipLink?? && client?? && client.baseUrl?has_content>
            <a class="pk-btn pk-btn-quiet" href="${(client.baseUrl)!''}">Back to Punchable</a>
        </#if>
        <a class="pk-link" href="${(url.loginRestartFlowUrl)!(url.loginUrl)}">Start sign-in again</a>
    </#if>
</@layout.registrationLayout>
