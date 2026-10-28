<#import "template.ftl" as layout>
<#-- Keycloak's generic "done / next step" page (after a reset link is used, an email is confirmed, ...). -->
<#if messageHeader??><#global pageTitle = msg(messageHeader)><#else><#global pageTitle = "Done"></#if>
<@layout.registrationLayout displayMessage=false; section>
    <#if section = "form">
        <p class="pk-hint">${kcSanitize((message.summary)!'')?no_esc}<#if requiredActions??> <#list requiredActions as a><b>${kcSanitize(msg("requiredAction.${a}"))?no_esc}</b><#sep>, </#list>.</#if></p>
        <#if skipLink??>
        <#elseif pageRedirectUri?has_content>
            <a id="backToApplication" class="pk-btn" href="${pageRedirectUri}">Back to Punchable</a>
        <#elseif actionUri?has_content>
            <a id="proceedWithAction" class="pk-btn" href="${actionUri}">Continue</a>
        <#elseif (client.baseUrl)?has_content>
            <a id="backToApplication" class="pk-btn" href="${(client.baseUrl)!''}">Back to Punchable</a>
        </#if>
    </#if>
</@layout.registrationLayout>
