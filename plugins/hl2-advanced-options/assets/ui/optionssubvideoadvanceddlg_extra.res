// HL2-RS additions to the installed Video > Advanced dialog (resource/OptionsSubVideoAdvancedDlg.res),
// appended to it when it loads. They sit in the free band under the Field of view slider (the
// divider is at ypos 352). Labels are temporary placeholders (gameui::localize::placeholders);
// slider ranges come from assets/ui/tuning.json, so no minvalue/maxvalue here. The labels
// `associate` their sliders: an unbound label near the buttons would otherwise count as part of
// the dialog's footer and move down with it.
"resource/OptionsSubVideoAdvancedDlg_Extra.res"
{
	"VerticalFovLabel"
	{
		"ControlName"		"Label"
		"fieldName"		"VerticalFovLabel"
		"associate"		"VerticalFovSlider"
		"xpos"		"176"
		"ypos"		"302"
		"wide"		"100"
		"tall"		"24"
		"visible"		"1"
		"enabled"		"1"
		"labelText"		"#HL2RS_VerticalFov"
		"textAlignment"		"west"
	}
	"VerticalFovValueLabel"
	{
		"ControlName"		"Label"
		"fieldName"		"VerticalFovValueLabel"
		"xpos"		"276"
		"ypos"		"302"
		"wide"		"32"
		"tall"		"24"
		"visible"		"1"
		"enabled"		"1"
		"labelText"		"0"
		"textAlignment"		"east"
	}
	"VerticalFovSlider"
	{
		"ControlName"		"CCvarSlider"
		"fieldName"		"VerticalFovSlider"
		"xpos"		"176"
		"ypos"		"322"
		"wide"		"140"
		"tall"		"30"
		"visible"		"1"
		"enabled"		"1"
		"allowoutofrange"		"0"
	}
	"ViewmodelFovLabel"
	{
		"ControlName"		"Label"
		"fieldName"		"ViewmodelFovLabel"
		"associate"		"ViewmodelFovSlider"
		"xpos"		"330"
		"ypos"		"302"
		"wide"		"100"
		"tall"		"24"
		"visible"		"1"
		"enabled"		"1"
		"labelText"		"#HL2RS_ViewmodelFov"
		"textAlignment"		"west"
	}
	"ViewmodelFovValueLabel"
	{
		"ControlName"		"Label"
		"fieldName"		"ViewmodelFovValueLabel"
		"xpos"		"430"
		"ypos"		"302"
		"wide"		"32"
		"tall"		"24"
		"visible"		"1"
		"enabled"		"1"
		"labelText"		"54"
		"textAlignment"		"east"
	}
	"ViewmodelFovSlider"
	{
		"ControlName"		"CCvarSlider"
		"fieldName"		"ViewmodelFovSlider"
		"xpos"		"330"
		"ypos"		"322"
		"wide"		"132"
		"tall"		"30"
		"visible"		"1"
		"enabled"		"1"
		"allowoutofrange"		"0"
	}
}
