<#import "template.ftl" as layout>
<#global pageTitle = (messageHeader)!"Done">
<@layout.registrationLayout displayMessage=false; section>
    <#if section = "form">
        <p class="pk-sub" style="margin:0">${kcSanitize((message.summary)!'')?no_esc}<#if requiredActions??> <#list requiredActions as a><b>${kcSanitize(msg("requiredAction.${a}"))?no_esc}</b><#sep>, </#list>.</#if></p>
        <#if skipLink??>
        <#elseif pageRedirectUri?has_content>
            <a class="pk-btn" href="${pageRedirectUri}">Back to Punchable</a>
        <#elseif actionUri?has_content>
            <a class="pk-btn" href="${actionUri}">Continue</a>
        <#elseif (client.baseUrl)?has_content>
            <a class="pk-btn" href="${(client.baseUrl)!''}">Back to Punchable</a>
        </#if>
    </#if>
</@layout.registrationLayout>
