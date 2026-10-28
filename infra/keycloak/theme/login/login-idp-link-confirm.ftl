<#import "template.ftl" as layout>
<#-- Shown if a first-broker flow ever reaches "confirm link". Punchable's flow (pk-google-link-only) never creates a
     user and auto-links, so only the link choice is offered (no "review profile"). The broker username is rendered in
     the form section so it is HTML-escaped (the subtitle is not). -->
<#global pageTitle = "Link ${idpDisplayName!'Google'} to Punchable?">
<@layout.registrationLayout displayInfo=false; section>
    <#if section = "form">
        <p class="pk-hint">There's already a Punchable account for <b>${(brokerContext.username)!'this address'}</b>. Link it and either way in works.</p>
        <form id="kc-register-form" action="${url.loginAction}" method="post" data-once>
            <button class="pk-btn" type="submit" name="submitAction" id="linkAccount" value="linkAccount" data-busy="Linking…">Link ${idpDisplayName!'Google'}</button>
            <a class="pk-link pk-center" href="${url.loginRestartFlowUrl}">Cancel</a>
        </form>
    </#if>
</@layout.registrationLayout>
