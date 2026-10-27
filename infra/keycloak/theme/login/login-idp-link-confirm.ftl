<#import "template.ftl" as layout>
<#-- Shown if a first-broker flow ever reaches "confirm link". Punchable's flow (pk-google-link-only) never creates a user,
     so only the link choice is offered. -->
<#global pageTitle = "Link ${idpDisplayName!'Google'} to Punchable?">
<@layout.registrationLayout displayInfo=false; section>
    <#if section = "form">
        <p class="pk-sub" style="margin:0">There's already a Punchable account for <b>${(brokerContext.username)!'this address'}</b>. Link them and you can sign in with either.</p>
        <form id="kc-register-form" action="${url.loginAction}" method="post">
            <button class="pk-btn" type="submit" name="submitAction" id="linkAccount" value="linkAccount">Link ${idpDisplayName!'Google'}</button>
            <a class="pk-link" href="${url.loginRestartFlowUrl}">Cancel and go back to sign in</a>
        </form>
    </#if>
</@layout.registrationLayout>
