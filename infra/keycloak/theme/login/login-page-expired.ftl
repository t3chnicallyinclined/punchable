<#import "template.ftl" as layout>
<#global pageTitle = "This page timed out">
<#global pageSubtitle = "Sign-in pages only stay open for a while. Nothing was lost.">
<@layout.registrationLayout displayInfo=false; section>
    <#if section = "form">
        <a class="pk-btn" id="loginRestartLink" href="${url.loginRestartFlowUrl}">Start again</a>
        <a class="pk-link pk-center" id="loginContinueLink" href="${url.loginAction}">Continue where I was</a>
    </#if>
</@layout.registrationLayout>
