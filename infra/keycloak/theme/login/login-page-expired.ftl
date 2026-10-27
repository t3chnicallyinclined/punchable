<#import "template.ftl" as layout>
<#global pageTitle = "This page timed out">
<#global pageSubtitle = "Sign-in pages only stay open for a while. Nothing was lost.">
<@layout.registrationLayout displayInfo=false; section>
    <#if section = "form">
        <a class="pk-btn" id="loginRestartLink" href="${url.loginRestartFlowUrl}">Start again</a>
        <a class="pk-link" id="loginContinueLink" href="${url.loginAction}">Try to pick up where I was</a>
    </#if>
</@layout.registrationLayout>
